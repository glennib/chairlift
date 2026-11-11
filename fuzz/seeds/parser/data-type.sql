6Tuple(a Nullable(String), b Map(String, Array(LowCardinality(String))), c AggregateFunction(quantiles(0.5, 0.9), UInt64), d Enum8('x' = 1), e JSON(a.b UInt32, SKIP c))
