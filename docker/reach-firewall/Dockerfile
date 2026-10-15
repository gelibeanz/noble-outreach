# Host firewall for reach-server (see reach-fw.sh). Runs in the host network namespace with NET_ADMIN only.
# Both iptables backends are installed; the script uses whichever one holds Docker's DOCKER-USER chain.
FROM alpine:3.22
RUN apk add --no-cache iptables iptables-legacy
COPY reach-fw.sh /usr/local/bin/reach-fw
RUN chmod +x /usr/local/bin/reach-fw
ENTRYPOINT ["/usr/local/bin/reach-fw"]
CMD ["run"]
