drop table if exists a.b, c on cluster c sync;
truncate t;
rename table a to b, c to d;
optimize table t partition tuple() final deduplicate by a;
set a = 1, -- c
b = 2;
use d;
attach table t
(
  a UInt8
)
engine = Memory;
with x as (select 1) insert into t (a, b) settings a = 1 select * from x;
insert into t values
  (1),
  (2) -- last
;
