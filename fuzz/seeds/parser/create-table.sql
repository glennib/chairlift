0CREATE TABLE IF NOT EXISTS db.t ON CLUSTER '{cluster}'
(
    a UInt8 DEFAULT 1 CODEC(Delta, ZSTD(1)) COMMENT 'x',
    b String NOT NULL,
    c Nullable(DateTime64(3, 'UTC')) TTL c + INTERVAL 1 DAY,
    d Tuple(x Int8, y Array(LowCardinality(String))) EPHEMERAL,
    INDEX i a TYPE minmax GRANULARITY 1,
    PROJECTION p (SELECT a, count() GROUP BY a),
    CONSTRAINT k CHECK a > 0
)
ENGINE = ReplicatedMergeTree('/t/{shard}', '{replica}')
PARTITION BY toYYYYMM(c)
ORDER BY (a DESC, b)
TTL toDate(c) + INTERVAL 1 MONTH GROUP BY a SET b = any(b), toDate(c) + INTERVAL 1 YEAR TO VOLUME 'cold'
SETTINGS index_granularity = 8192, definer = 1
COMMENT 'table'
