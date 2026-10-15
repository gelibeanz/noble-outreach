#!/bin/sh
# Egress lockdown for the Halo: Reach server. reach-server sits alone on the Docker bridge "br-reach"
# (network reach-net in compose.yml). Everything it starts itself is dropped, except:
#   - HTTP to a reach-master on this host (TCP REACH_FW_MASTER_PORT, default 47880, on a host address such as
#     the Docker bridge gateway),
#   - with REACH_FW_HTTPS=1: HTTPS (TCP 443) to anywhere, for listing on a remote https:// master,
#   - DNS to the public resolvers in REACH_FW_DNS (reach-host resolves hostnames such as a tunnel address in
#     REACH_PUBLIC_ADDR before each heartbeat; Docker's embedded DNS forwards from the container's netns).
# Replies to traffic that came IN (players through the published UDP port, directly or via a tunnel agent's
# forward) still pass: conntrack already knows those connections. So: no LAN, no VPN peers, no other
# containers, no host services (SSH, web UIs), no internet beyond DNS (and HTTPS when allowed).
#
#   reach-fw run      apply now, re-check every 30 s (the reach-firewall service; survives Docker/rule resets)
#   reach-fw apply    apply once        reach-fw check   exit 0 if the rules are in place
#   reach-fw remove   take every rule out again (undo)
#
# Rules (iptables; filter table unless noted):
#   INPUT       -i br-reach -j REACH-FW-IN      (traffic to the host itself)
#   DOCKER-USER -i br-reach -j REACH-FW-FWD     (traffic routed onward: LAN, VPN peers, containers, internet)
#   DOCKER-USER -o br-reach -j REACH-FW-FWD     (traffic routed to reach-server)
#   REACH-FW-IN : ESTABLISHED,RELATED ACCEPT; tcp dport 47880 ACCEPT; DROP
#   REACH-FW-FWD: ESTABLISHED,RELATED ACCEPT; -o br-reach RETURN (new inbound: Docker's own rules decide, so
#                 only the published port gets in); tcp dport 47880 ctstate DNAT ACCEPT (<gateway>:47880 is
#                 DNATed by Docker to reach-master's container, on another bridge); udp/tcp 53 to each
#                 REACH_FW_DNS ACCEPT; [REACH_FW_HTTPS=1: tcp dport 443 ACCEPT]; DROP
#   nat POSTROUTING -j REACH-FW-NAT: -s REACH_FW_SUBNET udp/tcp 53 to each REACH_FW_DNS MASQUERADE
#                 [REACH_FW_HTTPS=1: tcp 443 MASQUERADE] (reach-net has masquerade off, so only these can leave
#                 for the internet)
#   The ACCEPTs skip Docker's isolation chains, which would drop the reach-net <-> docker0 master traffic.
set -eu
BR="${REACH_FW_BRIDGE:-br-reach}"
SUBNET="${REACH_FW_SUBNET:-10.213.47.0/28}"
MASTER_PORT="${REACH_FW_MASTER_PORT:-47880}"
DNS="${REACH_FW_DNS:-1.1.1.1 1.0.0.1}"
HTTPS="${REACH_FW_HTTPS:-0}"
ipt=""
log() { echo "[reach-fw] $*"; }

# Use the iptables backend that holds Docker's chains (legacy or nft, depending on the distribution).
detect() {
  ipt=""
  for c in iptables-legacy iptables-nft; do
    if $c -w -S DOCKER-USER >/dev/null 2>&1; then ipt="$c"; return 0; fi
  done
  return 1
}

rules_ok() {
  [ -n "$ipt" ] || return 1
  $ipt -w -C INPUT -i "$BR" -j REACH-FW-IN 2>/dev/null || return 1
  $ipt -w -C DOCKER-USER -i "$BR" -j REACH-FW-FWD 2>/dev/null || return 1
  $ipt -w -C DOCKER-USER -o "$BR" -j REACH-FW-FWD 2>/dev/null || return 1
  $ipt -w -C REACH-FW-IN -j DROP 2>/dev/null || return 1
  $ipt -w -C REACH-FW-FWD -p tcp --dport "$MASTER_PORT" -m conntrack --ctstate DNAT -j ACCEPT 2>/dev/null || return 1
  $ipt -w -C REACH-FW-FWD -j DROP 2>/dev/null || return 1
  $ipt -w -t nat -C POSTROUTING -j REACH-FW-NAT 2>/dev/null || return 1
  for d in $DNS; do
    $ipt -w -C REACH-FW-FWD -d "$d" -p udp --dport 53 -j ACCEPT 2>/dev/null || return 1
    $ipt -w -t nat -C REACH-FW-NAT -s "$SUBNET" -d "$d" -p udp --dport 53 -j MASQUERADE 2>/dev/null || return 1
  done
  if [ "$HTTPS" = 1 ]; then
    $ipt -w -C REACH-FW-FWD -p tcp --dport 443 -j ACCEPT 2>/dev/null || return 1
    $ipt -w -t nat -C REACH-FW-NAT -s "$SUBNET" -p tcp --dport 443 -j MASQUERADE 2>/dev/null || return 1
  fi
  # the jumps must come first: an earlier ACCEPT (e.g. Docker's own) would win otherwise
  [ "$($ipt -w -S INPUT | sed -n 2p)" = "-A INPUT -i $BR -j REACH-FW-IN" ] || return 1
  [ "$($ipt -w -S DOCKER-USER | sed -n 2,3p | sort | tr '\n' ' ')" = \
    "-A DOCKER-USER -i $BR -j REACH-FW-FWD -A DOCKER-USER -o $BR -j REACH-FW-FWD " ] || return 1
}

remove() {
  [ -n "$ipt" ] || return 0
  while $ipt -w -D INPUT -i "$BR" -j REACH-FW-IN 2>/dev/null; do :; done
  while $ipt -w -D DOCKER-USER -i "$BR" -j REACH-FW-FWD 2>/dev/null; do :; done
  while $ipt -w -D DOCKER-USER -o "$BR" -j REACH-FW-FWD 2>/dev/null; do :; done
  while $ipt -w -t nat -D POSTROUTING -j REACH-FW-NAT 2>/dev/null; do :; done
  for ch in REACH-FW-IN REACH-FW-FWD; do $ipt -w -F $ch 2>/dev/null || true; $ipt -w -X $ch 2>/dev/null || true; done
  $ipt -w -t nat -F REACH-FW-NAT 2>/dev/null || true; $ipt -w -t nat -X REACH-FW-NAT 2>/dev/null || true
}

apply() {
  [ -n "$ipt" ] || { log "no DOCKER-USER chain in iptables-legacy or iptables-nft (Docker not up yet?)"; return 1; }
  rules_ok && return 0
  remove
  $ipt -w -N REACH-FW-IN
  $ipt -w -A REACH-FW-IN -m conntrack --ctstate ESTABLISHED,RELATED -j ACCEPT
  $ipt -w -A REACH-FW-IN -p tcp --dport "$MASTER_PORT" -j ACCEPT
  $ipt -w -A REACH-FW-IN -j DROP
  $ipt -w -N REACH-FW-FWD
  $ipt -w -A REACH-FW-FWD -m conntrack --ctstate ESTABLISHED,RELATED -j ACCEPT
  $ipt -w -A REACH-FW-FWD -o "$BR" -j RETURN
  $ipt -w -A REACH-FW-FWD -p tcp --dport "$MASTER_PORT" -m conntrack --ctstate DNAT -j ACCEPT
  $ipt -w -t nat -N REACH-FW-NAT
  for d in $DNS; do
    for p in udp tcp; do
      $ipt -w -A REACH-FW-FWD -d "$d" -p $p --dport 53 -j ACCEPT
      $ipt -w -t nat -A REACH-FW-NAT -s "$SUBNET" -d "$d" -p $p --dport 53 -j MASQUERADE
    done
  done
  if [ "$HTTPS" = 1 ]; then
    $ipt -w -A REACH-FW-FWD -p tcp --dport 443 -j ACCEPT
    $ipt -w -t nat -A REACH-FW-NAT -s "$SUBNET" -p tcp --dport 443 -j MASQUERADE
  fi
  $ipt -w -A REACH-FW-FWD -j DROP
  $ipt -w -I INPUT 1 -i "$BR" -j REACH-FW-IN
  $ipt -w -I DOCKER-USER 1 -o "$BR" -j REACH-FW-FWD
  $ipt -w -I DOCKER-USER 1 -i "$BR" -j REACH-FW-FWD
  $ipt -w -t nat -I POSTROUTING 1 -j REACH-FW-NAT
  log "applied ($ipt, bridge $BR, master port $MASTER_PORT, dns $DNS, https $HTTPS)"
}

case "${1:-run}" in
  run)
    trap 'exit 0' TERM INT
    while true; do
      detect || true   # each pass: Docker may have been restarted
      apply || true
      sleep 30 & wait $!
    done ;;
  apply) detect || true; apply ;;
  check) detect && rules_ok ;;
  remove) detect || true; remove; log "removed" ;;
  *) echo "usage: reach-fw run|apply|check|remove" >&2; exit 2 ;;
esac
