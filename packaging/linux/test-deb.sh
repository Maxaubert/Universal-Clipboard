#!/bin/sh
# Install and uninstall test for the .deb (plan 8.5), run as root in a throwaway container or
# CI runner, never on a real desktop:
#
#   docker run --rm -v "$PWD:/src" -w /src ubuntu:24.04 sh packaging/linux/test-deb.sh dist/universal-clipboard_X_amd64.deb
#
# Checks: install, both programs run, the CLI name rule (uc, then uclip when another "uc"
# exists), a test-profile agent starts and answers "uc status", remove leaves no files of the
# package and no CLI link, purge prints the advice, and the supervisor of a running agent
# exits when the binary is gone.
set -eu
DEB=${1:?deb}
export DEBIAN_FRONTEND=noninteractive
fail() { echo "FAIL: $*" >&2; exit 1; }

apt-get update -qq >/dev/null
# 1. Plain install.
apt-get install -y -qq "./$DEB" >/tmp/install.log 2>&1 || { cat /tmp/install.log; fail "install"; }
grep -q 'command line tool is "uc"' /tmp/install.log || fail "postinst message"
[ -x /usr/lib/universal-clipboard/uc-agent ] || fail "agent missing"
[ "$(readlink /usr/bin/uc)" = /usr/lib/universal-clipboard/uc ] || fail "uc link"
uc --version | grep -q "uc " || fail "uc --version"
uc-agent --version | grep -q "uc-agent " || fail "uc-agent --version"

# 2. A test-profile agent (fake clipboard, file key store, loopback) under the supervisor.
export UCLIP_PROFILE=debtest UCLIP_STATE_DIR=/tmp/uc-state UCLIP_TEST_KEYSTORE=file \
  UCLIP_TEST_LOOPBACK=127.0.0.1 UCLIP_NO_START=1
uc-agent --supervise >/tmp/agent.log 2>&1 &
SUP=$!
i=0
until uc status --json >/tmp/status.json 2>/dev/null; do
  i=$((i + 1)); [ $i -gt 100 ] && { cat /tmp/agent.log; fail "agent did not answer"; }
  sleep 0.1
done
grep -q '"state"' /tmp/status.json || fail "status"
PID=$(sed -n 's/.*"pid": *\([0-9]*\).*/\1/p' /tmp/status.json | head -n 1)
[ -n "$PID" ] || fail "no agent pid"

# 3. Remove: files and link gone, the advice printed.
apt-get remove -y -qq universal-clipboard >/tmp/remove.log 2>&1 || { cat /tmp/remove.log; fail "remove"; }
grep -q 'uc leave' /tmp/remove.log || fail "remove advice"
[ ! -e /usr/lib/universal-clipboard/uc-agent ] || fail "agent left behind"
[ ! -e /usr/bin/uc ] && [ ! -L /usr/bin/uc ] || fail "uc link left behind"
dpkg-query -W -f='${Status}' universal-clipboard 2>/dev/null | grep -q "ok installed" && fail "package still installed"
# The running agent keeps running until it exits; then the supervisor sees no binary and ends.
kill -9 "$PID"
i=0
while kill -0 $SUP 2>/dev/null; do
  i=$((i + 1)); [ $i -gt 100 ] && fail "supervisor still running after the binary is gone"
  sleep 0.1
done
grep -q "not installed any more" /tmp/agent.log || { cat /tmp/agent.log; fail "supervisor message"; }

# 4. Name rule: another "uc" exists, so ours is "uclip".
printf '#!/bin/sh\necho other\n' > /usr/local/bin/uc
chmod +x /usr/local/bin/uc
apt-get install -y -qq "./$DEB" >/tmp/install2.log 2>&1 || { cat /tmp/install2.log; fail "install 2"; }
grep -q 'named "uclip"' /tmp/install2.log || fail "uclip message"
[ "$(readlink /usr/bin/uclip)" = /usr/lib/universal-clipboard/uc ] || fail "uclip link"
[ "$(/usr/local/bin/uc)" = other ] || fail "the other uc was changed"
apt-get purge -y -qq universal-clipboard >/tmp/purge.log 2>&1 || fail "purge"
grep -q 'uc leave' /tmp/purge.log || fail "purge advice"
[ ! -e /usr/bin/uclip ] || fail "uclip link left behind"
[ -e /usr/local/bin/uc ] || fail "the other uc was removed"
echo "deb install/uninstall test: PASS"
