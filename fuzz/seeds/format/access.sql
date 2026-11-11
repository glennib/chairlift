grant select, alter update on db.t to a, b@'%' with grant option;
revoke grant option for select(a) on db.* from all except current_user;
create user if not exists u identified with plaintext_password by 'x' host local -- c
  default role all settings a = 1 min 0 max 2 readonly;
alter user u rename to v drop host local;
create quota q keyed by user_name for interval 1 hour max queries = 1, errors = 2 to r;
create row policy p on db.t as restrictive for select using a = 1 to all;
create settings profile p settings a = 1 inherit 'x';
drop row policy p on db.t;
set default role none to u;
show tables from system like 'x';
describe table file('a') format TSV;
