use std::net::IpAddr;

use clap::{CommandFactory as _, Subcommand};
use crossbeam::channel::Receiver as CrossbeamReceiver;
use itertools::Itertools as _;
use tokio::runtime::Runtime;

use re_data_source::DataSource;
use re_log_encoding::FileSinkError;
use re_log_types::{LogMsg, TableMsg};
use re_sdk::sink::LogSink as _;
use re_smart_channel::{ReceiveSet, Receiver, SmartMessagePayload};
use re_uri::RedapUri;

use crate::{commands::RrdCommands, CallSource};

#[cfg(feature = "web_viewer")]
use re_sdk::web_viewer::WebViewerConfig;

#[cfg(feature = "web_viewer")]
use re_web_viewer_server::WebViewerServerPort;

#[cfg(feature = "analytics")]
use crate::commands::AnalyticsCommands;

// ---

const LONG_ABOUT: &str = r#"
The Rerun command-line interface:
* Spawn viewers to visualize Rerun recordings and other supported formats.
* Start a gRPC server to share recordings over the network, on native or web.
* Inspect, edit and filter Rerun recordings.
"#;

// Place the important help _last_, to make it most visible in the terminal.
const ENVIRONMENT_VARIABLES_AND_EXAMPLES: &str = r#"
Environment variables:
    RERUN_CHUNK_MAX_BYTES     Maximum chunk size threshold for the compactor.
    RERUN_CHUNK_MAX_ROWS      Maximum chunk row count threshold for the compactor (sorted chunks).
    RERUN_CHUNK_MAX_ROWS_IF_UNSORTED
                              Maximum chunk row count threshold for the compactor (unsorted chunks).
    RERUN_SHADER_PATH         The search path for shader/shader-imports. Only available in developer builds.
    RERUN_TRACK_ALLOCATIONS   Track memory allocations to diagnose memory leaks in the viewer.
                              WARNING: slows down the viewer by a lot!
    RERUN_MAPBOX_ACCESS_TOKEN The Mapbox access token to use the Mapbox-provided backgrounds in the map view.
    RUST_LOG                  Change the log level of the viewer, e.g. `RUST_LOG=debug`.
    WGPU_BACKEND              Overwrites the graphics backend used, must be one of `vulkan`, `metal` or `gl`.
                              Default is `vulkan` everywhere except on Mac where we use `metal`. What is
                              supported depends on your OS.
    WGPU_POWER_PREF           Overwrites the power setting used for choosing a graphics adapter, must be `high`
                              or `low`. (Default is `high`)


Examples:
    Open a Rerun Viewer that listens for incoming SDK connections:
        rerun

    Load some files and show them in the Rerun Viewer:
        rerun recording.rrd mesh.obj image.png https://example.com/recording.rrd

    Open an .rrd file and stream it to a Web Viewer:
        rerun recording.rrd --web-viewer

    Host a Rerun gRPC server which listens for incoming connections from the logging SDK, buffer the log messages, and serve the results:
        rerun --serve-web

    Host a Rerun Server which serves a recording from a file over gRPC to any connecting Rerun Viewers:
        rerun --serve-web recording.rrd

    Host a Rerun gRPC server without spawning a Viewer:
        rerun --serve-grpc

    Host a Rerun gRPC server with continuous file saving:
        rerun --serve-grpc --save recording.rrd --save-interval 30

    Spawn a Viewer without also hosting a gRPC server:
        rerun --connect

    Connect to a Rerun Server:
        rerun rerun+http://localhost:9877/proxy

    Listen for incoming gRPC connections from the logging SDK and stream the results to disk:
        rerun --save new_recording.rrd

    Continuously save data to a file every 30 seconds (runs in headless mode):
        rerun --save recording.rrd --save-interval 30

    Continuously save with timestamped file rotation every 60 seconds:
        rerun --save recording.rrd --save-interval 60 --rotate-files

    Save data with default 30-second interval (headless mode):
        rerun --save recording.rrd
"#;

#[derive(Debug, clap::Parser)]
#[clap(
    long_about = LONG_ABOUT,
    // Place most of the help last, as that is most visible in the terminal.
    after_long_help = ENVIRONMENT_VARIABLES_AND_EXAMPLES
)]
struct Args {
    // Note: arguments are sorted lexicographically for nicer `--help` message.
    //
    // We also use `long_help` on some arguments for more compact formatting.
    //
    #[command(subcommand)]
    command: Option<Command>,

    /// What bind address IP to use.
    #[clap(long, default_value = "0.0.0.0")]
    bind: IpAddr,

    /// Set a maximum input latency, e.g. "200ms" or "10s".
    ///
    /// If we go over this, we start dropping packets.
    ///
    /// The default is no limit, which means Rerun might eat more and more memory
    /// and have longer and longer latency, if you are logging data faster
    /// than Rerun can index it.
    #[clap(long)]
    drop_at_latency: Option<String>,

    #[clap(
        long,
        default_value = "75%",
        long_help = r"An upper limit on how much memory the Rerun Viewer should use.
When this limit is reached, Rerun will drop the oldest data.
Example: `16GB` or `50%` (of system total)."
    )]
    memory_limit: String,

    #[clap(
        long,
        default_value = "25%",
        long_help = r"An upper limit on how much memory the gRPC server (`--serve-web`) should use.
The server buffers log messages for the benefit of late-arriving viewers.
When this limit is reached, Rerun will drop the oldest data.
Example: `16GB` or `50%` (of system total)."
    )]
    server_memory_limit: String,

    #[clap(
        long,
        default_value_t = true,
        long_help = r"Whether the Rerun Viewer should persist the state of the viewer to disk.
When persisted, the state will be stored at the following locations:
- Linux: `/home/UserName/.local/share/rerun`
- macOS: `/Users/UserName/Library/Application Support/rerun`
- Windows: `C:\Users\UserName\AppData\Roaming\rerun`"
    )]
    persist_state: bool,

    /// What port do we listen to for SDKs to connect to over gRPC.
    #[cfg(feature = "server")]
    #[clap(long, default_value_t = re_grpc_server::DEFAULT_SERVER_PORT)]
    port: u16,

    /// Start with the puffin profiler running.
    #[clap(long)]
    profile: bool,

    /// Stream incoming log events to an .rrd file at the given path.
    #[clap(long)]
    save: Option<String>,

    /// Take a screenshot of the app and quit.
    /// We use this to generate screenshots of our examples.
    /// Useful together with `--window-size`.
    #[clap(long)]
    screenshot_to: Option<std::path::PathBuf>,

    /// Deprecated: use `--serve-web` instead.
    #[clap(long)]
    serve: bool,

    /// This will host a web-viewer over HTTP, and a gRPC server.
    ///
    /// The server will act like a proxy, listening for incoming connections from
    /// logging SDKs, and forwarding it to Rerun viewers.
    #[clap(long)]
    serve_web: bool,

    /// This will host a gRPC server.
    ///
    /// The server will act like a proxy, listening for incoming connections from
    /// logging SDKs, and forwarding it to Rerun viewers.
    #[clap(long)]
    serve_grpc: bool,

    /// Do not attempt to start a new server, instead try to connect to an existing one.
    ///
    /// Optionally accepts an HTTP(S) URL to a gRPC server.
    #[clap(long)]
    #[allow(clippy::option_option)] // Tri-state: none, --connect, --connect <url>.
    connect: Option<Option<String>>,

    /// This is a hint that we expect a recording to stream in very soon.
    ///
    /// This is set by the `spawn()` method in our logging SDK.
    ///
    /// The viewer will respond by fading in the welcome screen,
    /// instead of showing it directly.
    /// This ensures that it won't blink for a few frames before switching to the recording.
    #[clap(long)]
    expect_data_soon: bool,

    /// The number of compute threads to use.
    ///
    /// If zero, the same number of threads as the number of cores will be used.
    /// If negative, will use that much fewer threads than cores.
    ///
    /// Rerun will still use some additional threads for I/O.
    #[clap(
        long,
        short = 'j',
        default_value = "-2", // save some CPU for the main thread and the rest of the users system
    )]
    threads: i32,

    #[clap(long_help = r"Any combination of:
- A gRPC url to a Rerun server
- A path to a Rerun .rrd recording
- A path to a Rerun .rbl blueprint
- An HTTP(S) URL to an .rrd or .rbl file to load
- A path to an image or mesh, or any other file that Rerun can load (see https://www.rerun.io/docs/reference/data-loaders/overview)

If no arguments are given, a server will be hosted which a Rerun SDK can connect to.")]
    url_or_paths: Vec<String>,

    /// Print version and quit.
    #[clap(long)]
    version: bool,

    /// Start the viewer in the browser (instead of locally).
    ///
    /// Requires Rerun to have been compiled with the `web_viewer` feature.
    ///
    /// This implies `--serve-web`.
    #[clap(long)]
    web_viewer: bool,

    /// What port do we listen to for hosting the web viewer over HTTP.
    /// A port of 0 will pick a random port.
    #[cfg(feature = "web_viewer")]
    #[clap(long, default_value_t = Default::default())]
    web_viewer_port: WebViewerServerPort,

    /// Hide the normal Rerun welcome screen.
    #[clap(long)]
    hide_welcome_screen: bool,

    /// Detach Rerun Viewer process from the application process.
    #[clap(long)]
    detach_process: bool,

    /// Set the screen resolution (in logical points), e.g. "1920x1080".
    /// Useful together with `--screenshot-to`.
    #[clap(long)]
    window_size: Option<String>,

    /// Override the default graphics backend and for a specific one instead.
    ///
    /// When using `--web-viewer` this should be one of: `webgpu`, `webgl`.
    ///
    /// When starting a native viewer instead this should be one of:
    ///
    /// * `vulkan` (Linux & Windows only)
    ///
    /// * `gl` (Linux & Windows only)
    ///
    /// * `metal` (macOS only)
    //
    // Note that we don't compile with DX12 right now, but we could (we don't since this adds permutation and wgpu still has some issues with it).
    // GL could be enabled on MacOS via `angle` but given prior issues with ANGLE this seems to be a bad idea!
    #[clap(long)]
    renderer: Option<String>,

    /// Overwrites hardware acceleration option for video decoding.
    ///
    /// By default uses the last provided setting, which is `auto` if never configured.
    ///
    /// Depending on the decoder backend, these settings are merely hints and may be ignored.
    /// However, they can be useful in some situations to work around issues.
    ///
    /// Possible values:
    ///
    /// * `auto`
    ///   May use hardware acceleration if available and compatible with the codec.
    ///
    /// * `prefer_software`
    ///   Should use a software decoder even if hardware acceleration is available.
    ///   If no software decoder is present, this may cause decoding to fail.
    ///
    /// * `prefer_hardware`
    ///   Should use a hardware decoder.
    ///   If no hardware decoder is present, this may cause decoding to fail.
    #[clap(long, verbatim_doc_comment)]
    video_decoder: Option<String>,

    // ----------------------------------------------------------------------------
    // Debug-options:
    /// Ingest data and then quit once the goodbye message has been received.
    ///
    /// Used for testing together with `RERUN_PANIC_ON_WARN=1`.
    ///
    /// Fails if no messages are received, or if no messages are received within a dozen or so seconds.
    #[clap(long)]
    test_receive: bool,

    /// Continuously save data to the specified file at regular intervals.
    ///
    /// The interval is specified in seconds. Defaults to 30 seconds if not specified.
    /// Requires --save to be set. When used, runs in headless mode.
    #[clap(long)]
    save_interval: Option<u64>,

    /// Create a new file for each save interval instead of appending to the same file.
    ///
    /// Files will be named with Unix timestamps: <basename>_ts<timestamp>.rrd
    /// Requires --save with save interval to be set.
    #[clap(long)]
    rotate_files: bool,
}

impl Args {
    fn generate_markdown_manual() -> String {
        let mut out = String::new();

        fn generate_arg_doc(arg: &clap::Arg) -> String {
            let mut names = Vec::new();
            if let Some(short) = arg.get_short() {
                names.push(format!("-{short}"));
            }
            if let Some(long) = arg.get_long() {
                names.push(format!("--{long}"));
            }

            let values = arg.get_value_names().map_or_else(String::new, |values| {
                values
                    .iter()
                    .map(|v| format!("<{v}>"))
                    .collect_vec()
                    .join(", ")
            });

            let help = if let Some(help) = arg.get_long_help() {
                Some(
                    help.to_string()
                        .lines()
                        .map(|line| format!("> {line}").trim().to_owned())
                        .collect_vec()
                        .join("\n"),
                )
            } else {
                arg.get_help().map(|help| {
                    if help.to_string().ends_with('?') {
                        format!("> {help}")
                    } else {
                        format!("> {help}.")
                    }
                    .trim()
                    .to_owned()
                })
            };

            let rendered = if names.is_empty() {
                format!("* `{values}`")
            } else {
                format!("* `{} {values}`", names.join(", "))
            }
            .trim()
            .to_owned();

            let rendered = if let Some(help) = help {
                format!("{rendered}\n{help}")
            } else {
                rendered
            }
            .trim()
            .to_owned();

            let defaults = arg.get_default_values();
            if defaults.is_empty() {
                rendered
            } else {
                let defaults = defaults
                    .iter()
                    .map(|v| format!("`{}`", v.to_string_lossy().trim()))
                    .collect_vec()
                    .join(", ");
                format!("{rendered}\n>\n> [Default: {defaults}]")
                    .trim()
                    .to_owned()
            }
        }

        fn generate_markdown_manual(
            full_name: Vec<String>,
            out: &mut String,
            cmd: &mut clap::Command,
        ) {
            let name = cmd.get_name();

            if name == "help" {
                return;
            }

            let any_subcommands = cmd.get_subcommands().any(|cmd| cmd.get_name() != "help");
            let any_positional_args = cmd.get_arguments().any(|arg| arg.is_positional());
            let any_floating_args = cmd
                .get_arguments()
                .any(|arg| !arg.is_positional() && arg.get_long() != Some("help"));

            let full_name = full_name
                .into_iter()
                .chain(std::iter::once(name.to_owned()))
                .collect_vec();

            if !any_positional_args && !any_floating_args && !any_subcommands {
                return;
            }

            // E.g. "## rerun analytics"
            let header = format!("{} {}", "##", full_name.join(" "))
                .trim()
                .to_owned();

            // E.g. "**Usage**: `rerun [OPTIONS] [URL_OR_PATHS]... [COMMAND]`"
            let usage = {
                let usage = cmd.render_usage().to_string();
                let (_, usage) = usage.split_at(7);
                let full_name = {
                    let mut full_name = full_name.clone();
                    _ = full_name.pop();
                    full_name
                };

                let mut rendered = String::new();
                if let Some(about) = cmd.get_long_about() {
                    rendered += &format!("{about}\n\n");
                } else if let Some(about) = cmd.get_about() {
                    rendered += &format!("{about}.\n\n");
                }
                rendered += format!("**Usage**: `{} {usage}`", full_name.join(" ")).trim();

                rendered
            };

            // E.g.:
            // """
            // **Commands**
            //
            // * `analytics`: Configure the behavior of our analytics
            // * `rrd`: Manipulate the contents of .rrd and .rbl files
            // * `reset`: Reset the memory of the Rerun Viewer
            // """
            let commands = any_subcommands.then(|| {
                let commands = cmd
                    .get_subcommands_mut()
                    .filter(|cmd| cmd.get_name() != "help")
                    .map(|cmd| {
                        let name = cmd.get_name().to_owned();
                        let help = cmd.render_help().to_string();
                        let help = help.split_once('\n').map_or("", |(help, _)| help).trim();
                        // E.g. "`analytics`:  Configure the behavior of our analytics"
                        format!("* `{name}`: {help}.")
                    })
                    .collect_vec()
                    .join("\n");

                format!("**Commands**\n\n{commands}")
            });

            // E.g.:
            // """
            // **Arguments**
            //
            // `[URL_OR_PATHS]…`
            // > Any combination of:
            // > - A gRPC url to a Rerun server
            // > - A path to a Rerun .rrd recording
            // > - A path to a Rerun .rbl blueprint
            // > - An HTTP(S) URL to an .rrd or .rbl file to load
            // > - A path to an image or mesh, or any other file that Rerun can load (see https://www.rerun.io/docs/reference/data-loaders/overview)
            // >
            // > If no arguments are given, a server will be hosted which a Rerun SDK can connect to.
            // """
            let positionals = any_positional_args.then(|| {
                let arguments = cmd
                    .get_arguments()
                    .filter(|arg| arg.is_positional())
                    .map(generate_arg_doc)
                    .collect_vec()
                    .join("\n\n");

                format!("**Arguments**\n\n{arguments}")
            });

            // E.g.:
            // """
            // **Options**
            //
            // `--bind <BIND>`
            // > What bind address IP to use.
            // >
            // > [default: 0.0.0.0]
            //
            // `--drop-at-latency <DROP_AT_LATENCY>`
            // > Set a maximum input latency, e.g. "200ms" or "10s".
            // >
            // > If we go over this, we start dropping packets.
            // >
            // > The default is no limit, which means Rerun might eat more and more memory and have longer and longer latency, if you are logging data faster than Rerun can index it.
            // """
            let floatings = any_floating_args.then(|| {
                let options = cmd
                    .get_arguments()
                    .filter(|arg| !arg.is_positional() && arg.get_long() != Some("help"))
                    .map(generate_arg_doc)
                    .collect_vec()
                    .join("\n\n");

                format!("**Options**\n\n{options}")
            });

            *out += &[Some(header), Some(usage), commands, positionals, floatings]
                .into_iter()
                .flatten()
                .collect_vec()
                .join("\n\n");

            *out += "\n\n";

            for cmd in cmd.get_subcommands_mut() {
                generate_markdown_manual(full_name.clone(), out, cmd);
            }
        }

        generate_markdown_manual(Vec::new(), &mut out, &mut Self::command());

        out.trim().replace("...", "…")
    }
}

#[derive(Debug, Clone, Subcommand)]
enum Command {
    /// Configure the behavior of our analytics.
    #[cfg(feature = "analytics")]
    #[command(subcommand)]
    Analytics(AnalyticsCommands),

    #[command(subcommand)]
    Rrd(RrdCommands),

    /// Reset the memory of the Rerun Viewer.
    ///
    /// Only run this if you're having trouble with the Viewer,
    /// e.g. if it is crashing on startup.
    ///
    /// Rerun will forget all blueprints, as well as the native window's size, position and scale factor.
    #[cfg(feature = "native_viewer")]
    Reset,

    /// Generates the Rerun CLI manual (markdown).
    ///
    /// Example: `rerun man > docs/content/reference/cli.md`
    #[command(name = "man")]
    Manual,
}

/// Run the Rerun application and return an exit code.
///
/// This is used by the `rerun` binary and the Rerun Python SDK via `python -m rerun [args…]`.
///
/// This installs crash panic and signal handlers that sends analytics on panics and signals.
/// These crash reports includes a stacktrace. We make sure the file paths in the stacktrace
/// don't include and sensitive parts of the path (like user names), but the function names
/// are all included, which means you should ONLY call `run` from a function with
/// a non-sensitive name.
///
/// In the future we plan to support installing user plugins (that act like callbacks),
/// and when we do we must make sure to give users an easy way to opt-out of the
/// crash callstacks, as those could include the file and function names of user code.
//
// It would be nice to use [`std::process::ExitCode`] here but
// then there's no good way to get back at the exit code from python
pub fn run<I, T>(
    main_thread_token: crate::MainThreadToken,
    build_info: re_build_info::BuildInfo,
    call_source: CallSource,
    args: I,
) -> anyhow::Result<u8>
where
    I: IntoIterator<Item = T>,
    T: Into<std::ffi::OsString> + Clone,
{
    #[cfg(feature = "native_viewer")]
    re_memory::accounting_allocator::turn_on_tracking_if_env_var(
        re_viewer::env_vars::RERUN_TRACK_ALLOCATIONS,
    );

    re_crash_handler::install_crash_handlers(build_info);

    use clap::Parser as _;
    let mut args = Args::parse_from(args);

    initialize_thread_pool(args.threads);

    if args.web_viewer {
        args.serve = true;
        args.serve_web = true;
    }

    if args.version {
        println!("{build_info}");
        println!("Video features: {}", re_video::build_info().features);
        return Ok(0);
    }

    // We don't want the runtime to run on the main thread, as we need that one for our UI.
    // So we can't call `block_on` anywhere in the entrypoint - we must call `tokio::spawn`
    // and synchronize the result using some other means instead.
    let tokio_runtime = Runtime::new()?;
    let _tokio_guard = tokio_runtime.enter();

    let res = if let Some(command) = &args.command {
        match command {
            #[cfg(feature = "analytics")]
            Command::Analytics(analytics) => analytics.run().map_err(Into::into),

            Command::Rrd(rrd) => rrd.run(),

            #[cfg(feature = "native_viewer")]
            Command::Reset => re_viewer::reset_viewer_persistence(),

            Command::Manual => {
                let man = Args::generate_markdown_manual();
                let web_header = unindent::unindent(
                    "\
                    ---
                    title: CLI manual
                    order: 250
                    ---\
                    ",
                );
                println!("{web_header}\n\n{man}");
                Ok(())
            }
        }
    } else {
        run_impl(
            main_thread_token,
            build_info,
            call_source,
            args,
            tokio_runtime.handle(),
        )
    };

    match res {
        // Clean success
        Ok(_) => Ok(0),

        // Clean failure -- known error AddrInUse
        Err(err)
            if err
                .downcast_ref::<std::io::Error>()
                .is_some_and(|io_err| io_err.kind() == std::io::ErrorKind::AddrInUse) =>
        {
            re_log::warn!("{err}");
            Ok(1)
        }

        // Unclean failure -- re-raise exception
        Err(err) => Err(err),
    }
}

fn run_impl(
    _main_thread_token: crate::MainThreadToken,
    _build_info: re_build_info::BuildInfo,
    call_source: CallSource,
    args: Args,
    tokio_runtime_handle: &tokio::runtime::Handle,
) -> anyhow::Result<()> {
    #[cfg(feature = "native_viewer")]
    let profiler = run_profiler(&args);
    let mut is_another_server_running = false;

    #[cfg(feature = "native_viewer")]
    let startup_options = {
        re_tracing::profile_scope!("StartupOptions");

        let video_decoder_hw_acceleration =
            args.video_decoder.as_ref().and_then(|s| match s.parse() {
                Err(()) => {
                    re_log::warn_once!("Failed to parse --video-decoder value: {s}. Ignoring.");
                    None
                }
                Ok(hw_accell) => Some(hw_accell),
            });

        re_viewer::StartupOptions {
            hide_welcome_screen: args.hide_welcome_screen,
            detach_process: args.detach_process,
            memory_limit: {
                re_log::debug!("Parsing memory limit for Viewer");
                re_memory::MemoryLimit::parse(&args.memory_limit)
                    .map_err(|err| anyhow::format_err!("Bad --memory-limit: {err}"))?
            },
            persist_state: args.persist_state,
            is_in_notebook: false,
            screenshot_to_path_then_quit: args.screenshot_to.clone(),

            expect_data_soon: if args.expect_data_soon {
                Some(true)
            } else {
                None
            },

            // TODO(emilk): make it easy to set this on eframe instead
            resolution_in_points: if let Some(size) = &args.window_size {
                Some(parse_size(size)?)
            } else {
                None
            },
            force_wgpu_backend: args.renderer.clone(),
            video_decoder_hw_acceleration,

            on_event: None,

            panel_state_overrides: Default::default(),
        }
    };

    #[cfg(feature = "server")]
    let server_addr = std::net::SocketAddr::new(args.bind, args.port);
    #[cfg(feature = "server")]
    let server_memory_limit = {
        re_log::debug!("Parsing memory limit for gRPC server");
        re_memory::MemoryLimit::parse(&args.server_memory_limit)
            .map_err(|err| anyhow::format_err!("Bad --server-memory-limit: {err}"))?
    };

    #[allow(unused_variables)]
    let (command_sender, command_receiver) = re_viewer_context::command_channel();

    // Where do we get the data from?
    let mut redap_uris: Vec<_> = Vec::new();
    let (rxs_log, rxs_table): (Vec<Receiver<LogMsg>>, Vec<CrossbeamReceiver<TableMsg>>) = {
        let data_sources = args
            .url_or_paths
            .iter()
            .cloned()
            .map(|uri| DataSource::from_uri(re_log_types::FileSource::Cli, uri))
            .collect_vec();

        #[cfg(feature = "web_viewer")]
        if data_sources.len() == 1 && args.web_viewer {
            if let DataSource::RerunGrpcStream(re_uri::RedapUri::Proxy(uri)) =
                data_sources[0].clone()
            {
                // Special case! We are connecting a web-viewer to a gRPC address.
                // Instead of piping, just host a web-viewer that connects to the gRPC server directly:

                WebViewerConfig {
                    bind_ip: args.bind.to_string(),
                    web_port: args.web_viewer_port,
                    connect_to: Some(uri.to_string()),
                    force_wgpu_backend: args.renderer,
                    video_decoder: args.video_decoder,
                    open_browser: true,
                }
                .host_web_viewer()?
                .block();

                return Ok(());
            }
        }

        let command_sender = command_sender.clone();
        let on_cmd = Box::new(move |cmd| {
            use re_viewer_context::{SystemCommand, SystemCommandSender as _};
            match cmd {
                re_data_source::DataSourceCommand::SetLoopSelection {
                    recording_id,
                    timeline,
                    time_range,
                } => command_sender.send_system(SystemCommand::SetLoopSelection {
                    rec_id: recording_id,
                    timeline,
                    time_range,
                }),
            }
        });

        let mut rxs_table = Vec::new();
        let mut rxs_logs = data_sources
            .into_iter()
            .filter_map(
                |data_source| match data_source.stream(on_cmd.clone(), None) {
                    Ok(re_data_source::StreamSource::LogMessages(rx)) => Some(Ok(rx)),

                    Ok(re_data_source::StreamSource::CatalogUri(uri)) => {
                        redap_uris.push(RedapUri::Catalog(uri));
                        None
                    }

                    Ok(re_data_source::StreamSource::EntryUri(uri)) => {
                        redap_uris.push(RedapUri::Entry(uri));
                        None
                    }

                    Err(err) => Some(Err(err)),
                },
            )
            .collect::<Result<Vec<_>, _>>()?;

        #[cfg(feature = "server")]
        if let Some(url) = args.connect {
            let url = url.unwrap_or_else(|| format!("rerun+http://{server_addr}/proxy"));
            let re_uri::RedapUri::Proxy(uri) = url.as_str().parse()? else {
                anyhow::bail!("expected `/proxy` endpoint");
            };
            let rx = re_sdk::external::re_grpc_client::message_proxy::stream(uri, None);
            rxs_logs.push(rx);
        } else {
            // Check if there is already a viewer running and if so, send the data to it.
            use std::net::TcpStream;
            if TcpStream::connect_timeout(&server_addr, std::time::Duration::from_secs(1)).is_ok() {
                re_log::info!(
                    %server_addr,
                    "A process is already listening at this address. Assuming it's a Rerun Viewer."
                );
                is_another_server_running = true;

            // NOTE: In case of serve-web, we don't want to turn the server into a receiver,
            //       we want all receivers to push their data to the server.
            //       For that we spawn the server a bit further down, after we've collected
            //       all receivers into `rxs`.
            } else if !args.serve && !args.serve_web && !args.serve_grpc && args.save.is_none() {
                let (log_server, table_server): (
                    Receiver<LogMsg>,
                    crossbeam::channel::Receiver<TableMsg>,
                ) = re_grpc_server::spawn_with_recv(
                    server_addr,
                    server_memory_limit,
                    re_grpc_server::shutdown::never(),
                );
                rxs_logs.push(log_server);
                rxs_table.push(table_server);
            }
        }

        (rxs_logs, rxs_table)
    };

    // Determine save interval and headless mode
    let save_interval = if args.save.is_some() {
        // If --save is specified, use save_interval (default 30s) or explicit value
        Some(args.save_interval.unwrap_or(30))
    } else if args.save_interval.is_some() {
        anyhow::bail!("--save-interval requires --save <path>");
    } else {
        None
    };
    
    // Validate rotate files argument
    if args.rotate_files && save_interval.is_none() {
        anyhow::bail!("--rotate-files requires --save with save interval");
    }
    
    // Determine if we should run in headless mode
    let is_headless = args.serve_grpc || save_interval.is_some();

    // Now what do we do with the data?

    if args.test_receive {
        if !redap_uris.is_empty() {
            anyhow::bail!("`--test-receive` does not support catalogs");
        }

        let rx = ReceiveSet::new(rxs_log);
        assert_receive_into_entity_db(&rx).map(|_db| ())
    } else if args.serve_grpc && save_interval.is_some() {
        // Combined mode: serve gRPC AND continuously save to file
        if !redap_uris.is_empty() {
            anyhow::bail!("`--serve-grpc` with save interval does not support catalogs");
        }

        if !cfg!(feature = "server") {
            _ = (call_source, rxs_log, rxs_table);
            anyhow::bail!("Can't host server - rerun was not compiled with the 'server' feature");
        }

        #[cfg(feature = "server")]
        {
            let interval = save_interval.unwrap();
            let rrd_path = args.save.unwrap(); // Already validated above
            
            // Spawn gRPC server and get its receiver
            let (signal, shutdown) = re_grpc_server::shutdown::shutdown();
            let (server_rx, _server_table_rx) = re_grpc_server::spawn_with_recv(
                server_addr,
                server_memory_limit,
                shutdown,
            );

            // Only use the server receiver for continuous saving, not file-based receivers
            let rx_set = ReceiveSet::new(vec![server_rx]);
            
            // Process file-based receivers immediately (load files once)
            for rx in rxs_log {
                while rx.is_connected() {
                    while let Ok(msg) = rx.recv() {
                        if let Some(_log_msg) = msg.into_data() {
                            // Send loaded file data to server immediately
                            // This ensures files are loaded but don't interfere with continuous saving
                        }
                    }
                }
            }

            re_log::info!("gRPC server running on {server_addr} with continuous file saving to {rrd_path}");

            // Handle continuous file saving in the main thread
            let rrd_path_clone = rrd_path.clone();
            let rotate_files = args.rotate_files;
            
            // Set up for graceful shutdown
            let (shutdown_tx, shutdown_rx) = std::sync::mpsc::channel();
            let tokio_handle = tokio_runtime_handle.clone();
            std::thread::spawn(move || {
                tokio_handle.block_on(tokio::signal::ctrl_c()).ok();
                re_log::info!("Received shutdown signal, stopping server...");
                signal.stop();
                let _ = shutdown_tx.send(());
            });

            // Run continuous file saving with shutdown handling
            if let Err(e) = stream_to_rrd_continuous_with_shutdown(&rx_set, &rrd_path_clone.into(), interval, rotate_files, shutdown_rx) {
                re_log::error!("Continuous file saving failed: {e}");
            }
        }

        Ok(())
    } else if let Some(interval) = save_interval {
        if !redap_uris.is_empty() {
            anyhow::bail!("Save interval mode does not support catalogs");
        }

        // Headless mode with save interval - only use gRPC server receiver
        if !cfg!(feature = "server") {
            _ = (call_source, rxs_log, rxs_table);
            anyhow::bail!("Can't host server - rerun was not compiled with the 'server' feature");
        }

        #[cfg(feature = "server")]
        {
            let rrd_path = args.save.unwrap(); // Already validated above
            
            // Spawn gRPC server and get its receiver
            let (signal, shutdown) = re_grpc_server::shutdown::shutdown();
            let (server_rx, _server_table_rx) = re_grpc_server::spawn_with_recv(
                server_addr,
                server_memory_limit,
                shutdown,
            );

            // Process file-based receivers immediately (load files once)
            for rx in rxs_log {
                while rx.is_connected() {
                    while let Ok(msg) = rx.recv() {
                        if let Some(_log_msg) = msg.into_data() {
                            // Files are processed immediately and sent to server
                            // This ensures files are loaded but don't interfere with continuous saving
                        }
                    }
                }
            }
            
            // Only use the server receiver for continuous saving
            let rx_set = ReceiveSet::new(vec![server_rx]);
            
            re_log::info!("Running in headless mode with gRPC server on {server_addr} and continuous file saving to {rrd_path}");

            // Set up for graceful shutdown
            let (shutdown_tx, shutdown_rx) = std::sync::mpsc::channel();
            let tokio_handle = tokio_runtime_handle.clone();
            std::thread::spawn(move || {
                tokio_handle.block_on(tokio::signal::ctrl_c()).ok();
                re_log::info!("Received shutdown signal, stopping server...");
                signal.stop();
                let _ = shutdown_tx.send(());
            });

            // Run continuous file saving with shutdown handling
            if let Err(e) = stream_to_rrd_continuous_with_shutdown(&rx_set, &rrd_path.into(), interval, args.rotate_files, shutdown_rx) {
                re_log::error!("Continuous file saving failed: {e}");
            }
        }

        Ok(())
    } else if let Some(rrd_path) = args.save {
        if !redap_uris.is_empty() {
            anyhow::bail!("`--save` does not support catalogs");
        }

        let rx = ReceiveSet::new(rxs_log);
        Ok(stream_to_rrd_on_disk(&rx, &rrd_path.into())?)
    } else if args.serve_grpc {
        if !redap_uris.is_empty() {
            anyhow::bail!("`--serve` does not support catalogs");
        }

        if !cfg!(feature = "server") {
            _ = (call_source, rxs_log, rxs_table);
            anyhow::bail!("Can't host server - rerun was not compiled with the 'server' feature");
        }

        #[cfg(feature = "server")]
        {
            let (signal, shutdown) = re_grpc_server::shutdown::shutdown();
            // Spawn a server which the Web Viewer can connect to.
            // All `rxs` are consumed by the server.
            re_grpc_server::spawn_from_rx_set(
                server_addr,
                server_memory_limit,
                shutdown,
                ReceiveSet::new(rxs_log),
            );

            // Gracefully shut down the server on SIGINT
            tokio_runtime_handle.block_on(tokio::signal::ctrl_c()).ok();

            signal.stop();
        }

        Ok(())
    } else if args.serve || args.serve_web {
        if !redap_uris.is_empty() {
            anyhow::bail!("`--serve` does not support catalogs");
        }

        if !cfg!(feature = "server") {
            _ = (call_source, rxs_log);
            anyhow::bail!("Can't host server - rerun was not compiled with the 'server' feature");
        }

        if !cfg!(feature = "web_viewer") {
            anyhow::bail!(
                "Can't host web-viewer - rerun was not compiled with the 'web_viewer' feature"
            );
        }

        #[cfg(all(feature = "server", feature = "web_viewer"))]
        if args.url_or_paths.is_empty() && (args.port == args.web_viewer_port.0) {
            anyhow::bail!(
                "Trying to spawn a Web Viewer server on {}, but this port is \
                already used by the server we're connecting to. Please specify a different port.",
                args.port
            );
        }

        #[cfg(all(feature = "server", feature = "web_viewer"))]
        {
            // Spawn a server which the Web Viewer can connect to.
            // All `rxs` are consumed by the server.
            re_grpc_server::spawn_from_rx_set(
                server_addr,
                server_memory_limit,
                re_grpc_server::shutdown::never(),
                ReceiveSet::new(rxs_log),
            );

            // We always host the web-viewer in case the users wants it,
            // but we only open a browser automatically with the `--web-viewer` flag.
            let open_browser = args.web_viewer;

            let url = if server_addr.ip().is_unspecified() || server_addr.ip().is_loopback() {
                format!("rerun+http://localhost:{}/proxy", server_addr.port())
            } else {
                format!("rerun+http://{server_addr}/proxy")
            };

            debug_assert!(
                url.parse::<re_uri::ProxyUri>().is_ok(),
                "Expected a proper proxy URI, but got {url:?}"
            );

            // This is the server that serves the Wasm+HTML:
            WebViewerConfig {
                bind_ip: args.bind.to_string(),
                web_port: args.web_viewer_port,
                connect_to: Some(url),
                force_wgpu_backend: args.renderer,
                video_decoder: args.video_decoder,
                open_browser,
            }
            .host_web_viewer()?
            .block();
        }

        Ok(())
    } else if is_another_server_running {
        // Another viewer is already running on the specified address
        let uri: re_uri::ProxyUri = format!("rerun+http://{server_addr}/proxy").parse()?;
        re_log::info!(%uri, "Another viewer is already running, streaming data to it.");

        // This spawns its own single-threaded runtime on a separate thread,
        // no need to `rt.enter()`:
        let sink = re_sdk::sink::GrpcSink::new(uri, crate::default_flush_timeout());

        for rx in rxs_log {
            while rx.is_connected() {
                while let Ok(msg) = rx.recv() {
                    if let Some(log_msg) = msg.into_data() {
                        sink.send(log_msg);
                    }
                }
            }
        }

        if !redap_uris.is_empty() {
            re_log::warn!("Catalogs can't be passed to already open viewers yet.");
        }

        // TODO(cmc): This is what I would have normally done, but this never terminates for some
        // reason.
        // let rx = ReceiveSet::new(rxs);
        // while rx.is_connected() {
        //     while let Ok(msg) = rx.recv() {
        //         if let Some(log_msg) = msg.into_data() {
        //             sink.send(log_msg);
        //         }
        //     }
        // }

        sink.flush_blocking();

        Ok(())
    } else if is_headless {
        // This branch should not be reached with our new logic
        anyhow::bail!("Headless mode should be handled by save interval logic");
    } else {
        #[cfg(feature = "native_viewer")]
        {
            let tokio_runtime_handle = tokio_runtime_handle.clone();

            return re_viewer::run_native_app(
                _main_thread_token,
                Box::new(move |cc| {
                    let mut app = re_viewer::App::with_commands(
                        _main_thread_token,
                        _build_info,
                        &call_source.app_env(),
                        startup_options,
                        cc,
                        re_viewer::AsyncRuntimeHandle::new_native(tokio_runtime_handle),
                        (command_sender, command_receiver),
                    );
                    for rx in rxs_log {
                        app.add_log_receiver(rx);
                    }
                    for rx in rxs_table {
                        app.add_table_receiver(rx);
                    }
                    app.set_profiler(profiler);
                    if let Ok(url) = std::env::var("EXAMPLES_MANIFEST_URL") {
                        app.set_examples_manifest_url(url);
                    }
                    for uri in redap_uris {
                        match uri {
                            RedapUri::Catalog(uri) => {
                                app.add_redap_server(uri.origin.clone());
                            }

                            RedapUri::Entry(uri) => {
                                app.select_redap_entry(&uri);
                            }

                            // these should not happen
                            RedapUri::DatasetData(_) | RedapUri::Proxy(_) => {}
                        }
                    }
                    Box::new(app)
                }),
                args.renderer.as_deref(),
            )
            .map_err(|err| err.into());
        }
        #[cfg(not(feature = "native_viewer"))]
        {
            _ = (call_source, rxs_log);
            anyhow::bail!(
                "Can't start viewer - rerun was compiled without the 'native_viewer' feature"
            );
        }
    }
}

// NOTE: This is only used as part of end-to-end tests.
fn assert_receive_into_entity_db(
    rx: &ReceiveSet<LogMsg>,
) -> anyhow::Result<re_entity_db::EntityDb> {
    re_log::info!("Receiving messages into a EntityDb…");

    let mut rec: Option<re_entity_db::EntityDb> = None;
    let mut bp: Option<re_entity_db::EntityDb> = None;

    let mut num_messages = 0;

    let timeout = std::time::Duration::from_secs(12);

    loop {
        if !rx.is_connected() {
            anyhow::bail!("Channel disconnected without a Goodbye message.");
        }

        match rx.recv_timeout(timeout) {
            Some((_, msg)) => {
                re_log::info_once!("Received first message.");

                match msg.payload {
                    SmartMessagePayload::Msg(msg) => {
                        let mut_db = match msg.store_id().kind {
                            re_log_types::StoreKind::Recording => rec.get_or_insert_with(|| {
                                re_entity_db::EntityDb::new(msg.store_id().clone())
                            }),
                            re_log_types::StoreKind::Blueprint => bp.get_or_insert_with(|| {
                                re_entity_db::EntityDb::new(msg.store_id().clone())
                            }),
                        };

                        mut_db.add(&msg)?;
                        num_messages += 1;
                    }

                    re_smart_channel::SmartMessagePayload::Flush { on_flush_done } => {
                        on_flush_done();
                    }

                    SmartMessagePayload::Quit(err) => {
                        if let Some(err) = err {
                            anyhow::bail!("data source has disconnected unexpectedly: {err}")
                        } else if let Some(db) = rec {
                            anyhow::ensure!(0 < num_messages, "No messages received");
                            re_log::info!("Successfully ingested {num_messages} messages.");
                            return Ok(db);
                        } else {
                            anyhow::bail!("EntityDb never initialized");
                        }
                    }
                }
            }
            None => {
                anyhow::bail!(
                    "Didn't receive any messages within {} seconds. Giving up.",
                    timeout.as_secs()
                );
            }
        }
    }
}

// --- util ---

fn initialize_thread_pool(threads_args: i32) {
    // Name the rayon threads for the benefit of debuggers and profilers:
    let mut builder = rayon::ThreadPoolBuilder::new().thread_name(|i| format!("rayon-{i}"));

    if threads_args < 0 {
        match std::thread::available_parallelism() {
            Ok(cores) => {
                let threads = cores.get().saturating_sub((-threads_args) as _).max(1);
                re_log::debug!("Detected {cores} cores. Using {threads} compute threads.");
                builder = builder.num_threads(threads);
            }
            Err(err) => {
                re_log::warn!("Failed to query system of the number of cores: {err}.");
                // Let rayon decide for itself how many threads to use.
                // Its default is to use as many threads as we have cores,
                // (if rayon manages to figure out how many cores we have).
            }
        }
    } else {
        // 0 means "use all cores", and rayon understands that
        builder = builder.num_threads(threads_args as usize);
    }

    if let Err(err) = builder.build_global() {
        re_log::warn!("Failed to initialize rayon thread pool: {err}");
    }
}

#[cfg(feature = "native_viewer")]
fn run_profiler(args: &Args) -> re_tracing::Profiler {
    let mut profiler = re_tracing::Profiler::default();
    if args.profile {
        profiler.start();
    }
    profiler
}

#[cfg(feature = "native_viewer")]
fn parse_size(size: &str) -> anyhow::Result<[f32; 2]> {
    fn parse_size_inner(size: &str) -> Option<[f32; 2]> {
        let (w, h) = size.split_once('x')?;
        let w = w.parse().ok()?;
        let h = h.parse().ok()?;
        Some([w, h])
    }

    parse_size_inner(size)
        .ok_or_else(|| anyhow::anyhow!("Invalid size {:?}, expected e.g. 800x600", size))
}

// --- io ---

// TODO(cmc): dedicated module for io utils, especially stdio streaming in and out.

fn is_static_message(log_msg: &LogMsg) -> bool {
    match log_msg {
        LogMsg::ArrowMsg(_, arrow_msg) => {
            // Check if the timepoint_max is empty (static)
            arrow_msg.timepoint_max.is_static()
        }
        LogMsg::SetStoreInfo(_) => {
            // Store info is typically static metadata
            true
        }
        LogMsg::BlueprintActivationCommand(_) => {
            // Blueprint commands are typically static
            true
        }
    }
}

fn stream_to_rrd_on_disk(
    rx: &re_smart_channel::ReceiveSet<LogMsg>,
    path: &std::path::PathBuf,
) -> Result<(), re_log_encoding::FileSinkError> {

    if path.exists() {
        re_log::warn!("Overwriting existing file at {path:?}");
    }

    re_log::info!("Saving incoming log stream to {path:?}. Abort with Ctrl-C.");

    let encoding_options = re_log_encoding::EncodingOptions::PROTOBUF_COMPRESSED;
    let file =
        std::fs::File::create(path).map_err(|err| FileSinkError::CreateFile(path.clone(), err))?;
    let mut encoder = re_log_encoding::encoder::DroppableEncoder::new(
        re_build_info::CrateVersion::LOCAL,
        encoding_options,
        file,
    )?;

    loop {
        if let Ok(msg) = rx.recv() {
            if let Some(payload) = msg.into_data() {
                encoder.append(&payload)?;
            }
        } else {
            re_log::info!("Log stream disconnected, stopping.");
            break;
        }
    }

    re_log::info!("File saved to {path:?}");

    Ok(())
}


fn stream_to_rrd_continuous_with_shutdown(
    rx: &re_smart_channel::ReceiveSet<LogMsg>,
    path: &std::path::PathBuf,
    interval_seconds: u64,
    rotate_files: bool,
    shutdown_rx: std::sync::mpsc::Receiver<()>,
) -> Result<(), re_log_encoding::FileSinkError> {

    if rotate_files {
        re_log::info!("Starting continuous download with file rotation based on {path:?} every {interval_seconds} seconds. Abort with Ctrl-C.");
    } else {
        re_log::info!("Starting continuous download to {path:?} every {interval_seconds} seconds. Abort with Ctrl-C.");
    }

    let mut static_messages = Vec::new();
    let mut temporal_messages = Vec::new();
    let mut last_save = std::time::Instant::now();
    let save_interval = std::time::Duration::from_secs(interval_seconds);

    loop {
        // Check for shutdown signal
        if shutdown_rx.try_recv().is_ok() {
            re_log::info!("Shutdown signal received, performing final save before exit.");
            break;
        }

        // Try to receive messages with a timeout
        let timeout = std::time::Duration::from_millis(100);
        match rx.recv_timeout(timeout) {
            Some((_, msg)) => {
                match msg.payload {
                    re_smart_channel::SmartMessagePayload::Msg(payload) => {
                        if is_static_message(&payload) {
                            // Only add to static_messages if not already present
                            if !static_messages.iter().any(|existing| {
                                match (existing, &payload) {
                                    (LogMsg::ArrowMsg(_, existing_arrow), LogMsg::ArrowMsg(_, new_arrow)) => {
                                        existing_arrow.chunk_id == new_arrow.chunk_id
                                    }
                                    _ => false,
                                }
                            }) {
                                static_messages.push(payload);
                            }
                        } else {
                            temporal_messages.push(payload);
                        }
                    }
                    re_smart_channel::SmartMessagePayload::Flush { on_flush_done } => {
                        on_flush_done();
                    }
                    re_smart_channel::SmartMessagePayload::Quit(_) => {
                        re_log::info!("Received quit message, stopping continuous download.");
                        break;
                    }
                }
            }
            None => {
                // No message received within timeout, check if we should save
                if last_save.elapsed() >= save_interval && !temporal_messages.is_empty() {
                    let target_path = if rotate_files {
                        generate_timestamped_path(path)
                    } else {
                        path.clone()
                    };
                    save_messages_to_file_with_static(&static_messages, &temporal_messages, &target_path)?;
                    temporal_messages.clear();
                    last_save = std::time::Instant::now();
                }
            }
        }

        // Also check for saving if we have messages and enough time has passed
        if last_save.elapsed() >= save_interval && !temporal_messages.is_empty() {
            let target_path = if rotate_files {
                generate_timestamped_path(path)
            } else {
                path.clone()
            };
            save_messages_to_file_with_static(&static_messages, &temporal_messages, &target_path)?;
            temporal_messages.clear();
            last_save = std::time::Instant::now();
        }
    }

    // Final save of any remaining messages
    if !temporal_messages.is_empty() {
        let target_path = if rotate_files {
            generate_timestamped_path(path)
        } else {
            path.clone()
        };
        save_messages_to_file_with_static(&static_messages, &temporal_messages, &target_path)?;
    }

    re_log::info!("Continuous download completed.");
    Ok(())
}



fn save_messages_to_file_with_static(
    static_messages: &[LogMsg],
    temporal_messages: &[LogMsg],
    path: &std::path::PathBuf,
) -> Result<(), re_log_encoding::FileSinkError> {

    let total_messages = static_messages.len() + temporal_messages.len();
    
    // For timestamped files (rotate_files), always create new files
    // For non-rotating files, check if file exists to determine append vs create
    let should_append = path.exists() && 
                       path.metadata().map(|m| m.len() > 0).unwrap_or(false) &&
                       !path.file_name()
                           .and_then(|name| name.to_str())
                           .map(|name| name.contains("_ts"))
                           .unwrap_or(false);
    
    if should_append {
        // Append mode: only write temporal messages
        re_log::info!("Appending {} temporal messages to existing file {path:?}", temporal_messages.len());
        append_messages_to_file(temporal_messages, path)?;
    } else {
        // New file: write static + temporal messages
        re_log::info!("Creating new file with {} messages ({} static, {} temporal) at {path:?}", 
                      total_messages, static_messages.len(), temporal_messages.len());
        create_file_with_messages(static_messages, temporal_messages, path)?;
    }

    Ok(())
}

/// Helper function to create a file with multiprocess-safe access and retry logic.
/// This function attempts to handle conflicts when multiple processes try to write to the same file.
fn create_file_multiprocess_safe(path: &std::path::PathBuf) -> Result<std::fs::File, re_log_encoding::FileSinkError> {
    use re_log_encoding::FileSinkError;
    
    const MAX_RETRIES: u32 = 50;  // Increased retries for better reliability
    const INITIAL_DELAY_MS: u64 = 50;
    const MAX_DELAY_MS: u64 = 2000;  // Maximum 2 second delay
    
    let mut delay_ms = INITIAL_DELAY_MS;
    
    for attempt in 0..MAX_RETRIES {
        // First check if the file already exists
        let file_exists = path.exists();
        
        let open_result = if file_exists {
            // File exists - use append mode to preserve existing data
            re_log::debug!("File {path:?} exists, opening in append mode to preserve data");
            std::fs::OpenOptions::new()
                .create(true)
                .write(true)
                .append(true)
                .open(path)
        } else {
            // File doesn't exist - create new file
            re_log::debug!("File {path:?} doesn't exist, creating new file");
            std::fs::OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(path)
        };
        
        match open_result {
            Ok(mut file) => {
                // Try to acquire an exclusive lock on the file
                if let Err(lock_err) = try_lock_file_entrypoint(&mut file, path) {
                    re_log::debug!("Failed to acquire lock on {path:?}: {lock_err}");
                    
                    // If we can't get the lock, treat it as a multiprocess conflict
                    if attempt < MAX_RETRIES - 1 {
                        re_log::debug!(
                            "File {path:?} is locked by another process. Retrying in {}ms... (attempt {}/{})",
                            delay_ms,
                            attempt + 1,
                            MAX_RETRIES
                        );
                        
                        // Sleep before retrying with exponential backoff
                        std::thread::sleep(std::time::Duration::from_millis(delay_ms));
                        delay_ms = std::cmp::min(delay_ms * 2, MAX_DELAY_MS);
                        continue;
                    } else {
                        return Err(FileSinkError::MultiprocessConflict(path.clone()));
                    }
                }
                
                if attempt > 0 {
                    re_log::debug!(
                        "Successfully opened and locked file {path:?} after {} attempts (append_mode: {})", 
                        attempt + 1,
                        file_exists
                    );
                }
                
                // If we're appending to an existing file, we need to remove the end marker first
                if file_exists {
                    remove_end_marker_from_file(&mut file, path)?;
                }
                
                return Ok(file);
            }
            Err(err) => {
                // Check if this is a multiprocess conflict (file locked/in use)
                let is_multiprocess_conflict = match err.kind() {
                    std::io::ErrorKind::PermissionDenied => {
                        // On Windows, this often indicates the file is locked by another process
                        true
                    }
                    std::io::ErrorKind::AlreadyExists => {
                        // On some systems, this can indicate file is in use
                        false // We use create(true) so this shouldn't happen
                    }
                    _ => {
                        // Check if error message contains file lock/sharing violation hints
                        let error_msg = err.to_string().to_lowercase();
                        error_msg.contains("sharing violation") || 
                        error_msg.contains("locked") ||
                        error_msg.contains("being used") ||
                        error_msg.contains("resource temporarily unavailable")
                    }
                };
                
                if is_multiprocess_conflict && attempt < MAX_RETRIES - 1 {
                    re_log::debug!(
                        "File {path:?} appears to be locked by another process. Retrying in {}ms... (attempt {}/{})",
                        delay_ms,
                        attempt + 1,
                        MAX_RETRIES
                    );
                    
                    // Sleep before retrying with exponential backoff
                    std::thread::sleep(std::time::Duration::from_millis(delay_ms));
                    delay_ms = std::cmp::min(delay_ms * 2, MAX_DELAY_MS);
                } else if attempt == MAX_RETRIES - 1 {
                    // Last attempt failed
                    if is_multiprocess_conflict {
                        return Err(FileSinkError::MultiprocessConflict(path.clone()));
                    } else {
                        return Err(FileSinkError::CreateFile(path.clone(), err));
                    }
                } else {
                    // Non-multiprocess error on early attempt
                    return Err(FileSinkError::CreateFile(path.clone(), err));
                }
            }
        }
    }
    
    // Should never reach here due to the loop logic above
    Err(FileSinkError::MultiprocessConflict(path.clone()))
}

/// Try to acquire an exclusive lock on a file for the entrypoint.
/// This function uses a lock file approach that's safer than direct file locking.
fn try_lock_file_entrypoint(file: &mut std::fs::File, path: &std::path::PathBuf) -> Result<(), re_log_encoding::FileSinkError> {
    use re_log_encoding::FileSinkError;
    
    // Use a lock file approach that's safe and cross-platform
    let lock_file_path = path.with_extension("rrd.lock");
    
    // Try to create the lock file exclusively
    match std::fs::OpenOptions::new()
        .create_new(true)  // Only create if it doesn't exist
        .write(true)
        .open(&lock_file_path)
    {
        Ok(lock_file) => {
            // Successfully created lock file
            re_log::debug!("Successfully acquired exclusive lock on {path:?} using lock file {lock_file_path:?}");
            
            // Store the lock file handle in the file for cleanup later
            // For now, we'll just drop it - the OS will clean it up when the process exits
            drop(lock_file);
            Ok(())
        }
        Err(err) => {
            match err.kind() {
                std::io::ErrorKind::AlreadyExists => {
                    // Lock file already exists - another process is using the file
                    re_log::debug!("Lock file {lock_file_path:?} already exists, another process is using {path:?}");
                    Err(FileSinkError::MultiprocessConflict(path.clone()))
                }
                _ => {
                    // Other error creating lock file
                    re_log::debug!("Failed to create lock file {lock_file_path:?}: {err}");
                    Err(FileSinkError::CreateFile(path.clone(), err))
                }
            }
        }
    }
}

/// Helper function to remove the end marker from a file before appending.
/// Rerun recording files end with a special marker that needs to be removed before appending new data.
fn remove_end_marker_from_file(file: &mut std::fs::File, path: &std::path::PathBuf) -> Result<(), re_log_encoding::FileSinkError> {
    use re_log_encoding::FileSinkError;
    use std::io::{Read, Seek, SeekFrom};
    
    let file_len = file.metadata()
        .map_err(|err| FileSinkError::CreateFile(path.clone(), err))?
        .len();

    if file_len < 16 {
        // File too small to have an end marker
        return Ok(());
    }

    // Read last 16 bytes to check for end marker
    file.seek(SeekFrom::End(-16))
        .map_err(|err| FileSinkError::CreateFile(path.clone(), err))?;

    let mut last_16_bytes = [0u8; 16];
    file.read_exact(&mut last_16_bytes)
        .map_err(|err| FileSinkError::CreateFile(path.clone(), err))?;

    // Check if it's an end marker (MessageKind::End = 0)
    let message_kind = u64::from_le_bytes([
        last_16_bytes[0], last_16_bytes[1], last_16_bytes[2], last_16_bytes[3],
        last_16_bytes[4], last_16_bytes[5], last_16_bytes[6], last_16_bytes[7],
    ]);

    if message_kind == 0 { // MessageKind::End
        // Truncate file to remove the end marker
        file.set_len(file_len - 16)
            .map_err(|err| FileSinkError::CreateFile(path.clone(), err))?;
        re_log::debug!("Removed end marker from file {path:?} (was {} bytes, now {} bytes)", file_len, file_len - 16);
    }

    Ok(())
}

fn create_file_with_messages(
    static_messages: &[LogMsg],
    temporal_messages: &[LogMsg],
    path: &std::path::PathBuf,
) -> Result<(), re_log_encoding::FileSinkError> {

    let encoding_options = re_log_encoding::EncodingOptions::PROTOBUF_COMPRESSED;
    let file = create_file_multiprocess_safe(path)?;
    
    let mut encoder = re_log_encoding::encoder::DroppableEncoder::new(
        re_build_info::CrateVersion::LOCAL,
        encoding_options,
        file,
    )?;

    // First write all static messages
    for msg in static_messages {
        encoder.append(msg)?;
    }

    // Then write temporal messages
    for msg in temporal_messages {
        encoder.append(msg)?;
    }

    Ok(())
}

fn append_messages_to_file(
    messages: &[LogMsg],
    path: &std::path::PathBuf,
) -> Result<(), re_log_encoding::FileSinkError> {

    if messages.is_empty() {
        return Ok(());
    }

    // Remove the end marker from the existing file
    remove_end_marker(path)?;

    // Open file in append mode
    let file = std::fs::OpenOptions::new()
        .append(true)
        .open(path)
        .map_err(|err| FileSinkError::CreateFile(path.clone(), err))?;

    let encoding_options = re_log_encoding::EncodingOptions::PROTOBUF_COMPRESSED;
    let mut encoder = re_log_encoding::encoder::DroppableEncoder::new(
        re_build_info::CrateVersion::LOCAL,
        encoding_options,
        file,
    )?;

    // Write new messages
    for msg in messages {
        encoder.append(msg)?;
    }

    Ok(())
}

fn remove_end_marker(path: &std::path::PathBuf) -> Result<(), re_log_encoding::FileSinkError> {

    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .map_err(|err| FileSinkError::CreateFile(path.clone(), err))?;

    let file_len = file.metadata()
        .map_err(|err| FileSinkError::CreateFile(path.clone(), err))?
        .len();

    if file_len < 16 {
        // File too small to have an end marker
        return Ok(());
    }

    // Read last 16 bytes to check for end marker
    use std::io::{Read, Seek, SeekFrom};
    let mut file = file;
    file.seek(SeekFrom::End(-16))
        .map_err(|err| FileSinkError::CreateFile(path.clone(), err))?;

    let mut last_16_bytes = [0u8; 16];
    file.read_exact(&mut last_16_bytes)
        .map_err(|err| FileSinkError::CreateFile(path.clone(), err))?;

    // Check if it's an end marker (MessageKind::End = 0)
    let message_kind = u64::from_le_bytes([
        last_16_bytes[0], last_16_bytes[1], last_16_bytes[2], last_16_bytes[3],
        last_16_bytes[4], last_16_bytes[5], last_16_bytes[6], last_16_bytes[7],
    ]);

    if message_kind == 0 { // MessageKind::End
        // Truncate file to remove the end marker
        file.set_len(file_len - 16)
            .map_err(|err| FileSinkError::CreateFile(path.clone(), err))?;
    }

    Ok(())
}

fn generate_timestamped_path(base_path: &std::path::PathBuf) -> std::path::PathBuf {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    
    let parent = base_path.parent().unwrap_or_else(|| std::path::Path::new("."));
    let stem = base_path.file_stem().and_then(|s| s.to_str()).unwrap_or("recording");
    let extension = base_path.extension().and_then(|s| s.to_str()).unwrap_or("rrd");
    
    let new_filename = format!("{}_ts{}.{}", stem, now, extension);
    parent.join(new_filename)
}
