//! Enodo fork: keep a proxy stream alive across server restarts and network drops.
//!
//! With a reconnect interval set (`rerun --reconnect-interval <seconds>`), a
//! `rerun+http://…/proxy` source never ends: when the connection fails or the server
//! closes it, the viewer waits the interval and tries again, indefinitely, and it also
//! waits for a server that is not up yet.
//!
//! Two things are filtered on the way back in, so a reconnect never disturbs what the
//! user already has:
//! - A chunk that was already delivered is dropped. A proxy that stayed up replays its
//!   whole buffer to a new client, and that would otherwise double every series.
//! - A blueprint for an application seen on an earlier connection is dropped, so a
//!   restarted process re-sending its default layout does not replace the one the user
//!   has arranged. Recording data (static included) still flows: a restarted process
//!   usually starts a new recording, which needs its own static data.

use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use re_log_types::{ApplicationId, LogMsg, StoreKind};

use crate::read::{StreamEnd, stream_async};

/// Milliseconds between attempts; 0 means reconnecting is off.
static RECONNECT_INTERVAL_MS: AtomicU64 = AtomicU64::new(0);

/// Makes every proxy stream opened after this call reconnect on its own.
///
/// `None` (the default) restores upstream behaviour: the stream ends with the connection.
pub fn set_reconnect_interval(interval: Option<Duration>) {
    let ms = interval.map_or(0, |d| (d.as_millis() as u64).max(1));
    RECONNECT_INTERVAL_MS.store(ms, Ordering::Relaxed);
}

pub(crate) fn reconnect_interval() -> Option<Duration> {
    match RECONNECT_INTERVAL_MS.load(Ordering::Relaxed) {
        0 => None,
        ms => Some(Duration::from_millis(ms)),
    }
}

/// Decides which messages of a new connection to forward. State spans all connections.
#[derive(Default)]
pub(crate) struct ReplayFilter {
    delivered_chunks: HashSet<re_tuid::Tuid>,

    /// Applications that have delivered anything, on any connection so far.
    seen_apps: HashSet<ApplicationId>,

    /// Applications seen before the current connection: their blueprints are not replaced.
    apps_before_connection: HashSet<ApplicationId>,

    /// Applications whose re-sent blueprint was already reported on this connection.
    reported_blueprints: HashSet<ApplicationId>,

    pub(crate) received_on_connection: bool,
}

impl ReplayFilter {
    pub(crate) fn start_connection(&mut self) {
        self.apps_before_connection = self.seen_apps.clone();
        self.reported_blueprints.clear();
        self.received_on_connection = false;
    }

    pub(crate) fn admit(&mut self, msg: &LogMsg) -> bool {
        self.received_on_connection = true;
        let store_id = msg.store_id();
        let app_id = store_id.application_id();

        if store_id.kind() == StoreKind::Blueprint && self.apps_before_connection.contains(app_id) {
            if self.reported_blueprints.insert(app_id.clone()) {
                re_log::info!(
                    "Ignoring the blueprint re-sent for {app_id} after reconnecting, to keep the current layout"
                );
            }
            return false;
        }

        if let LogMsg::ArrowMsg(_, arrow_msg) = msg
            && !self.delivered_chunks.insert(arrow_msg.chunk_id)
        {
            return false;
        }

        if !self.seen_apps.contains(app_id) {
            self.seen_apps.insert(app_id.clone());
        }
        true
    }
}

/// Streams from `uri` until the viewer drops the receiver, reconnecting every `interval`.
///
/// If the viewer drops the receiver while the server is down, this notices on the next
/// successful connection, so a closed source may keep polling until then.
pub(crate) async fn stream_reconnecting(
    uri: re_uri::ProxyUri,
    tx: re_log_channel::LogSender,
    interval: Duration,
) {
    let mut filter = ReplayFilter::default();
    let mut was_failing = false;

    loop {
        filter.start_connection();
        let result = stream_async(uri.clone(), &tx, Some(interval), &mut |msg| {
            if !filter.received_on_connection && was_failing {
                re_log::info!("Reconnected to {uri}");
            }
            filter.admit(msg)
        })
        .await;
        if filter.received_on_connection {
            was_failing = false;
        }

        match result {
            Ok(StreamEnd::ReceiverGone) => return,
            Ok(StreamEnd::ServerClosed) => {
                if !was_failing {
                    re_log::warn!(
                        "{uri} closed the stream; retrying every {}",
                        format_interval(interval)
                    );
                }
            }
            Err(err) => {
                if was_failing {
                    re_log::debug!("Still cannot reach {uri}: {err}");
                } else {
                    re_log::warn!(
                        "Cannot reach {uri}: {err}; retrying every {}",
                        format_interval(interval)
                    );
                }
            }
        }
        was_failing = true;

        tokio::time::sleep(interval).await;
    }
}

fn format_interval(interval: Duration) -> String {
    format!("{}s", interval.as_secs_f64())
}

#[cfg(test)]
mod tests {
    use re_log_types::{BlueprintActivationCommand, SetStoreInfo, StoreId, StoreInfo, StoreSource};

    use super::*;

    fn store_info(kind: StoreKind, app: &str, recording: &str) -> LogMsg {
        LogMsg::SetStoreInfo(SetStoreInfo {
            row_id: re_tuid::Tuid::new(),
            info: StoreInfo::new(StoreId::new(kind, app, recording), StoreSource::Unknown),
        })
    }

    fn activation(app: &str, recording: &str) -> LogMsg {
        LogMsg::BlueprintActivationCommand(BlueprintActivationCommand::make_active(StoreId::new(
            StoreKind::Blueprint,
            app,
            recording,
        )))
    }

    fn chunk(kind: StoreKind, app: &str, recording: &str, chunk_id: re_tuid::Tuid) -> LogMsg {
        let store_id = StoreId::new(kind, app, recording);
        let chunk = re_chunk::Chunk::builder_with_id(re_chunk::ChunkId::from_tuid(chunk_id), "a")
            .build()
            .unwrap();
        LogMsg::ArrowMsg(store_id, chunk.to_arrow_msg().unwrap())
    }

    #[test]
    fn first_connection_admits_everything() {
        let mut filter = ReplayFilter::default();
        filter.start_connection();
        assert!(filter.admit(&store_info(StoreKind::Recording, "app", "r1")));
        assert!(filter.admit(&store_info(StoreKind::Blueprint, "app", "b1")));
        assert!(filter.admit(&activation("app", "b1")));
    }

    #[test]
    fn reconnect_keeps_the_known_blueprint_but_takes_new_data() {
        let mut filter = ReplayFilter::default();
        filter.start_connection();
        assert!(filter.admit(&store_info(StoreKind::Recording, "app", "r1")));
        assert!(filter.admit(&store_info(StoreKind::Blueprint, "app", "b1")));

        // The process restarts: a new recording and a new default blueprint.
        filter.start_connection();
        assert!(filter.admit(&store_info(StoreKind::Recording, "app", "r2")));
        assert!(filter.admit(&chunk(
            StoreKind::Recording,
            "app",
            "r2",
            re_tuid::Tuid::new()
        )));
        assert!(!filter.admit(&store_info(StoreKind::Blueprint, "app", "b2")));
        assert!(!filter.admit(&chunk(
            StoreKind::Blueprint,
            "app",
            "b2",
            re_tuid::Tuid::new()
        )));
        assert!(!filter.admit(&activation("app", "b2")));
    }

    #[test]
    fn reconnect_takes_the_blueprint_of_a_new_application() {
        let mut filter = ReplayFilter::default();
        filter.start_connection();
        assert!(filter.admit(&store_info(StoreKind::Recording, "app", "r1")));

        filter.start_connection();
        assert!(filter.admit(&store_info(StoreKind::Blueprint, "other", "b1")));
        assert!(filter.admit(&activation("other", "b1")));
    }

    #[test]
    fn blueprint_of_the_first_connection_is_whole() {
        // Its own blueprint is not "re-sent" just because the app was seen earlier on the
        // same connection.
        let mut filter = ReplayFilter::default();
        filter.start_connection();
        assert!(filter.admit(&store_info(StoreKind::Recording, "app", "r1")));
        assert!(filter.admit(&store_info(StoreKind::Blueprint, "app", "b1")));
        assert!(filter.admit(&activation("app", "b1")));
    }

    #[test]
    fn replayed_chunks_are_dropped() {
        let id = re_tuid::Tuid::new();
        let mut filter = ReplayFilter::default();
        filter.start_connection();
        assert!(filter.admit(&chunk(StoreKind::Recording, "app", "r1", id)));

        filter.start_connection();
        assert!(!filter.admit(&chunk(StoreKind::Recording, "app", "r1", id)));
        assert!(filter.admit(&chunk(
            StoreKind::Recording,
            "app",
            "r1",
            re_tuid::Tuid::new()
        )));
    }

    #[test]
    fn interval_setting() {
        set_reconnect_interval(Some(Duration::from_millis(1500)));
        assert_eq!(reconnect_interval(), Some(Duration::from_millis(1500)));
        set_reconnect_interval(None);
        assert_eq!(reconnect_interval(), None);
    }
}
