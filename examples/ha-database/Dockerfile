# Minimal PostgreSQL + Patroni image for the Horizon HA database blueprint.
# There is no official Patroni image; this pins both PostgreSQL and Patroni.
#
#   docker build -t stellar-horizon-patroni:17-4.1.5 examples/ha-database
FROM postgres:17.11-alpine3.24

ARG PATRONI_VERSION=4.1.5

# tini reaps zombies and forwards SIGTERM so Patroni can release the leader lock
# on graceful shutdown (fast, clean failover on pod deletion).
RUN apk add --no-cache python3 py3-psycopg2 py3-yaml py3-psutil tini \
    && python3 -m venv --system-site-packages /opt/patroni \
    && /opt/patroni/bin/pip install --no-cache-dir "patroni==${PATRONI_VERSION}" \
    && mkdir -p /home/postgres/pgdata \
    && chown -R postgres:postgres /home/postgres

ENV PATH=/opt/patroni/bin:$PATH
USER postgres
WORKDIR /home/postgres
EXPOSE 5432 8008

ENTRYPOINT ["/sbin/tini", "--"]
CMD ["patroni", "/etc/patroni/patroni.yml"]
