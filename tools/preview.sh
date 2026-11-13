#!/usr/bin/env bash
# Serves one PR's preview on this machine's tailnet address, until the PR
# closes (preview.yml): the PR's web viewer and the demo recordings that
# tools/build_preview.sh made from it, plus a page listing the recordings.
#
# Recordings that must stay out of git (real robot logs, for example) can live
# in a folder on this machine, ~/rerun_preview_recordings
# (PREVIEW_PRIVATE_DIR). If it exists, every preview also serves a copy of it
# as recordings/private/, so a PR can say `Preview: private/<name>.rrd`.
#
#   tools/preview.sh up 12 <site>   serve the site built for PR #12 on port 9212
#   tools/preview.sh down 12        stop it and delete its files
set -euo pipefail

cmd="${1:?usage: preview.sh up <pr> <site> | down <pr>}"
pr="${2:?usage: preview.sh up <pr> <site> | down <pr>}"
[[ "$pr" =~ ^[0-9]+$ ]] || { echo "PR number expected, got $pr" >&2; exit 1; }
port=$(( ${PREVIEW_PORT_BASE:-9200} + pr ))
name="rerun-pr-$pr"
image="${PREVIEW_IMAGE:-nginxinc/nginx-unprivileged:1.29-alpine}"
dir="${PREVIEW_DIR:-$HOME/rerun_previews}/pr-$pr"
private="${PREVIEW_PRIVATE_DIR:-$HOME/rerun_preview_recordings}"

if [[ "$cmd" == down ]]; then
    docker rm -f "$name" > /dev/null 2>&1 || true
    rm -rf "$dir"
    echo "stopped the preview of PR #$pr"
    exit 0
fi
[[ "$cmd" == up ]] || { echo "unknown command $cmd" >&2; exit 1; }
site="${3:?usage: preview.sh up <pr> <site>}"
[[ -f "$site/index.html" ]] || { echo "$site has no index.html" >&2; exit 1; }

docker rm -f "$name" > /dev/null 2>&1 || true
rm -rf "$dir"
mkdir -p "$dir"
cp -a "$site" "$dir/site"
if [[ -d "$private" ]]; then
    [[ -e "$dir/site/recordings/private" ]] && { echo "a demo named private would hide $private" >&2; exit 1; }
    # -L copies the files behind symlinks, which a container can't follow out
    # of its mount.
    cp -RL "$private" "$dir/site/recordings/private"
    echo "also serving $private as recordings/private/"
fi

# A page that lists the recordings, each opening in the viewer. The viewer's
# ?url= needs an absolute URL (it reads recordings/x.rrd as a host name), and
# the page doesn't know which of the machine's names the browser used, so a
# script makes the links.
{
    echo "<!doctype html><meta charset=utf-8><meta name=viewport content='width=device-width'>"
    echo "<title>Rerun PR #$pr</title><style>body{font:15px system-ui;margin:2em;background:#0d1011;color:#cad8de}a{color:#7ab6ff}</style>"
    echo "<h1>Rerun PR #$pr</h1><ul>"
    (cd "$dir/site/recordings" && find . -name '*.rrd' | sed 's|^\./||' | sort) | while read -r rrd; do
        echo "<li><a data-rrd=\"recordings/$rrd\" href=\"./\">$rrd</a></li>"
    done
    echo "</ul><p><a href=./>Empty viewer</a></p>"
    echo "<script>for (const a of document.querySelectorAll('a[data-rrd]'))"
    echo "  a.href = './?url=' + encodeURIComponent(new URL(a.dataset.rrd, location.href).href);</script>"
} > "$dir/site/recordings.html"
chmod -R a+rX "$dir/site"

# nginx's default, plus compression: the viewer's wasm shrinks about 3x.
cat > "$dir/nginx.conf" <<'CONF'
server {
    listen 8080;
    root /usr/share/nginx/html;
    gzip on;
    gzip_types application/wasm application/javascript text/css application/json;
    gzip_min_length 1024;
}
CONF
chmod a+r "$dir/nginx.conf"

ip="$(tailscale ip -4 | head -n1)"
dns="$(tailscale status --json | python3 -c 'import json,sys; print(json.load(sys.stdin)["Self"]["DNSName"].rstrip("."))')"

docker run -d --name "$name" --restart unless-stopped \
    --read-only --tmpfs /tmp --cap-drop ALL --security-opt no-new-privileges \
    --label "rerun.preview=$pr" \
    -p "$ip:$port:8080" \
    -v "$dir/site:/usr/share/nginx/html:ro" \
    -v "$dir/nginx.conf:/etc/nginx/conf.d/default.conf:ro" \
    "$image" > /dev/null

for _ in $(seq 30); do
    curl -fsS -o /dev/null "http://$ip:$port/" && break
    sleep 1
done
curl -fsS -o /dev/null "http://$ip:$port/" || { docker logs "$name" >&2; exit 1; }

url="http://$dns:$port/"
echo "serving PR #$pr at $url"
if [[ -n "${GITHUB_OUTPUT:-}" ]]; then
    echo "url=$url" >> "$GITHUB_OUTPUT"
fi
exit 0
