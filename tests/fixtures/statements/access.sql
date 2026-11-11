-- Grants of privileges and of roles.
grant select on analytics.page_views to reader;
grant select, insert, alter update, alter delete on analytics.page_views_with_a_rather_long_name to analytics_writer, backfill_job with grant option;
GRANT ON CLUSTER '{cluster}' SELECT(visitor_id, site), dictGet ON *.* TO `report-user`@'10.%', currentUser() WITH REPLACE OPTION;
grant select on db.a, insert on db.b to writer_role;
grant reader, writer to alice with admin option;
grant current grants(select on db.*) to bob;
revoke grant option for select on db.* from all except alice;
revoke  admin option for writer from bob;

-- Users and roles.
create role if not exists events_writer settings max_threads = 4;
create user if not exists alice identified with sha256_password by 'p@ss; word' host ip '10.0.0.0/8', local default role events_writer default database analytics settings max_memory_usage = 10000000000 readonly;
create user bob identified with sha256_hash by '6b86b273ff34fce19d6b804eff5a3f5747ada4eaa22f1d49c01e52ddb7875b4b' salt 'salty';
alter user alice default role all except events_writer;
alter user if exists bob rename to robert drop host local;
alter role events_writer on cluster c rename to events_writers;
set role all except events_writer;
set default role none to alice, current_user;
drop user if exists alice, bob@'%' on cluster c;
drop role events_writers;

-- Quotas, row policies and settings profiles.
create quota if not exists q keyed by user_name for interval 1 hour max queries = 100, errors = 10, for randomized interval 1 day no limits to events_writer;
alter quota q for interval 1 month tracking only;
create row policy if not exists only_mine on analytics.page_views as restrictive for select using site = currentUser() and visitor_id != '' to all except admin;
create settings profile if not exists limited settings max_threads = 2 min 1 max 8 const, max_execution_time = 30 writable inherit 'default' to events_writer;
drop quota if exists q;
drop row policy if exists only_mine on analytics.page_views;
drop settings profile limited;
