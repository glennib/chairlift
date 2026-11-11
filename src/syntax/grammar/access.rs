//! Access control: `GRANT` and `REVOKE`; `CREATE`, `ALTER` and `DROP` of
//! users, roles, quotas, row policies and settings profiles; `SET ROLE` and
//! `SET DEFAULT ROLE`; as the parsers in ClickHouse's `src/Parsers/Access`
//! read them (`ParserGrantQuery`, `ParserCreateUserQuery` and its
//! relatives, `ParserDropAccessEntityQuery`, `ParserSetRoleQuery`,
//! `ParserRolesOrUsersSet`, `ParserSettingsProfileElement`).
//!
//! A `CREATE` or `ALTER` takes its clauses in any order, as ClickHouse's
//! loops over them do, so each is parsed with the clause fallback. What a
//! clause holds that chairlift does not model (the rarer authentication
//! methods, `ADD`/`DROP` in an `ALTER`, `VALID FOR`) is kept as written,
//! [`Opaque`](SyntaxKind::Opaque) by design; a password or hash is a string
//! literal either way, and comes out as written.
//!
//! # Keyword slots
//!
//! A privilege is a run of bare words that ClickHouse joins and looks up as
//! one phrase, case-insensitively (`AccessFlags::tryFromKeyword`), so its
//! words are keywords only when every one of them is a keyword word: `ALTER
//! UPDATE` is re-cased, `dictGet` and `SYSTEM RELOAD DICTIONARY` (`RELOAD`
//! is no keyword word) keep their case. In a list of users and roles,
//! `NONE` is always the keyword, `ALL`, `ANY` and `CURRENT_USER` only where
//! the statement allows them: `GRANT … TO all` grants to a user named
//! `all`, `REVOKE … FROM ALL` revokes from everyone. User, role, profile,
//! quota and policy names, setting names, authentication methods, quota
//! keys and quota resources are names and keep their case.

use crate::keyword::Keyword as K;
use crate::lexer::TokenKind as T;
use crate::syntax::SyntaxKind;
use crate::syntax::grammar::common;
use crate::syntax::grammar::exprs;
use crate::syntax::grammar::fallback;
use crate::syntax::grammar::fallback::Stop;
use crate::syntax::grammar::names;
use crate::syntax::parser::CompletedMarker;
use crate::syntax::parser::Parser;

/// The kinds of access entity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Entity {
    User,
    Role,
    Quota,
    RowPolicy,
    Profile,
}

/// The access entity whose keywords start at the `n`th token, and how many
/// words they are: `USER`, `ROLE`, `QUOTA`, `[ROW] POLICY`, `[SETTINGS]
/// PROFILE`.
pub(crate) fn entity(p: &Parser<'_>, n: usize) -> Option<(Entity, usize)> {
    match p.nth_keyword(n)? {
        K::User => Some((Entity::User, 1)),
        K::Role => Some((Entity::Role, 1)),
        K::Quota => Some((Entity::Quota, 1)),
        K::Policy => Some((Entity::RowPolicy, 1)),
        K::Row if p.nth_at_kw(n + 1, K::Policy) => Some((Entity::RowPolicy, 2)),
        K::Profile => Some((Entity::Profile, 1)),
        K::Settings if p.nth_at_kw(n + 1, K::Profile) => Some((Entity::Profile, 2)),
        _ => None,
    }
}

/// Whether the statement at the current token is one of this module's.
pub(crate) fn at_statement(p: &Parser<'_>) -> bool {
    match p.nth_keyword(0) {
        Some(K::Grant | K::Revoke) => true,
        Some(K::Create | K::Alter | K::Drop) => entity(p, 1).is_some(),
        Some(K::Set) => match p.nth_keyword(1) {
            Some(K::Role) => {
                !(p.nth_at(2, T::Equals) || p.nth_at(2, T::Comma) || p.nth(2).is_none())
            }
            Some(K::Default) => p.nth_at_kw(2, K::Role),
            _ => false,
        },
        _ => false,
    }
}

/// A statement of this module's, up to the end of the statement.
pub(crate) fn statement(p: &mut Parser<'_>) -> Option<()> {
    match p.nth_keyword(0) {
        Some(K::Grant | K::Revoke) => grant(p),
        Some(K::Set) => set_role(p),
        Some(K::Drop) => drop_entity(p),
        _ => create_or_alter(p),
    }
}

/// What a [`RoleSet`](SyntaxKind::RoleSet) may hold besides names.
#[derive(Clone, Copy, Default)]
pub(crate) struct Allow {
    pub(crate) all: bool,
    pub(crate) any: bool,
    pub(crate) current_user: bool,
}

/// Whether `ON CLUSTER` follows.
fn at_on_cluster(p: &Parser<'_>) -> bool {
    p.at_kws(&[K::On, K::Cluster])
}

/// Any `ON CLUSTER name`.
fn cluster(p: &mut Parser<'_>) -> Option<()> {
    if at_on_cluster(p) {
        common::on_cluster(p)?;
    }
    Some(())
}

// ---- GRANT and REVOKE -----------------------------------------------------

/// A [`GrantQuery`](SyntaxKind::GrantQuery).
fn grant(p: &mut Parser<'_>) -> Option<()> {
    let m = p.start();
    let revoke = p.at_kw(K::Revoke);
    p.bump_keyword();
    cluster(p)?;
    if revoke && !p.eat_kws(&[K::Grant, K::Option, K::For]) {
        p.eat_kws(&[K::Admin, K::Option, K::For]);
    }
    let to = if revoke { K::From } else { K::To };
    let before_grantees = move |p: &Parser<'_>| p.at_kw(to) || at_on_cluster(p);
    if !revoke && p.at_kws(&[K::Current, K::Grants]) {
        current_grants(p, &before_grantees)?;
    } else if at_privileges(p) {
        access_list(p, &before_grantees)?;
    } else {
        let roles = Allow {
            all: revoke,
            ..Allow::default()
        };
        role_set(p, roles, &before_grantees)?;
    }
    cluster(p)?;
    let grantees = p.start();
    p.expect_kw(to)?;
    let allow = Allow {
        all: revoke,
        any: false,
        current_user: true,
    };
    role_set(p, allow, &|p| at_on_cluster(p) || p.at_kw(K::With))?;
    grantees.complete(p, SyntaxKind::GranteesClause);
    cluster(p)?;
    if !revoke {
        if p.at_kws(&[K::With, K::Grant, K::Option]) || p.at_kws(&[K::With, K::Admin, K::Option]) {
            option(p);
        }
        if p.at_kws(&[K::With, K::Replace, K::Option]) {
            option(p);
        }
        cluster(p)?;
    }
    m.complete(p, SyntaxKind::GrantQuery);
    Some(())
}

/// A [`GrantOptionClause`](SyntaxKind::GrantOptionClause) of the three
/// keywords at the current token.
fn option(p: &mut Parser<'_>) {
    let m = p.start();
    for _ in 0..3 {
        p.bump_keyword();
    }
    m.complete(p, SyntaxKind::GrantOptionClause);
}

/// Whether privileges follow rather than roles: whether `ON`, other than
/// `ON CLUSTER`, comes before `TO` or `FROM` outside brackets, which is
/// what `parseAccessRightsElementsWithoutOptions` needs to succeed.
fn at_privileges(p: &Parser<'_>) -> bool {
    let mut n = 0;
    while p.nth(n).is_some() {
        match p.nth_keyword(n) {
            Some(K::On) if !p.nth_at_kw(n + 1, K::Cluster) => return true,
            Some(K::To | K::From) => return false,
            _ => {}
        }
        n += p.step_len(n);
    }
    false
}

/// An [`AccessList`](SyntaxKind::AccessList), with items that end at
/// `ends`.
fn access_list(p: &mut Parser<'_>, ends: Stop<'_>) -> Option<CompletedMarker> {
    let m = p.start();
    let shape = fallback::comma_separated(p, ends, &mut |p| access_element(p).map(drop));
    if shape.items == 0 {
        m.abandon(p);
        return p.expected("a privilege");
    }
    Some(m.complete(p, SyntaxKind::AccessList))
}

/// An [`AccessElement`](SyntaxKind::AccessElement): privileges, `ON` and
/// what they are on.
fn access_element(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    let m = p.start();
    let list = p.start();
    let shape = fallback::comma_separated(p, &|p| p.at_kw(K::On), &mut |p| privilege(p));
    if shape.items == 0 {
        list.abandon(p);
        m.abandon(p);
        return p.expected("a privilege");
    }
    list.complete(p, SyntaxKind::PrivilegeList);
    grant_on(p)?;
    Some(m.complete(p, SyntaxKind::AccessElement))
}

/// A [`Privilege`](SyntaxKind::Privilege): bare words up to `ON`, `TO` or
/// `FROM` (`parseAccessFlags`), then any columns in brackets.
fn privilege(p: &mut Parser<'_>) -> Option<()> {
    let mut n = 0;
    while p.nth_at(n, T::BareWord) && !matches!(p.nth_keyword(n), Some(K::On | K::To | K::From)) {
        n += 1;
    }
    if n == 0 {
        return p.expected("a privilege");
    }
    let keywords = (0..n).all(|i| !matches!(p.nth_keyword(i), None | Some(K::True | K::False)));
    let m = p.start();
    for _ in 0..n {
        if keywords {
            p.bump_as(SyntaxKind::Keyword);
        } else {
            p.bump();
        }
    }
    if p.at(T::OpeningRoundBracket) {
        fallback::delimited(
            p,
            SyntaxKind::ParenList,
            T::OpeningRoundBracket,
            T::ClosingRoundBracket,
            &mut |p| names::name(p).map(drop),
        )?;
    }
    m.complete(p, SyntaxKind::Privilege);
    Some(())
}

/// A [`GrantOn`](SyntaxKind::GrantOn): `ON` and a
/// [`GrantTarget`](SyntaxKind::GrantTarget).
fn grant_on(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    let m = p.start();
    p.expect_kw(K::On)?;
    grant_target(p)?;
    Some(m.complete(p, SyntaxKind::GrantOn))
}

/// A [`GrantTarget`](SyntaxKind::GrantTarget), as
/// `parseDatabaseAndTableNameOrAsterisks` reads it: `*`, `*.*`, or a name
/// with any `*`, then any `.` and `*` or a name with any `*`.
fn grant_target(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    let m = p.start();
    if p.eat(T::Asterisk) {
        if p.at(T::Dot) && p.nth_at(1, T::Asterisk) {
            p.bump();
            p.bump();
        }
    } else {
        names::name(p)?;
        p.eat(T::Asterisk);
        if p.at(T::Dot) {
            p.bump();
            if !p.eat(T::Asterisk) {
                names::name(p)?;
                p.eat(T::Asterisk);
            }
        }
    }
    Some(m.complete(p, SyntaxKind::GrantTarget))
}

/// A [`CurrentGrants`](SyntaxKind::CurrentGrants).
fn current_grants(p: &mut Parser<'_>, ends: Stop<'_>) -> Option<CompletedMarker> {
    let m = p.start();
    p.eat_kws(&[K::Current, K::Grants]);
    if p.at(T::OpeningRoundBracket) {
        p.expect(T::OpeningRoundBracket)?;
        access_list(p, ends)?;
        p.expect(T::ClosingRoundBracket)?;
    } else {
        grant_on(p)?;
    }
    Some(m.complete(p, SyntaxKind::CurrentGrants))
}

// ---- Users and roles ------------------------------------------------------

/// A [`RoleSet`](SyntaxKind::RoleSet) (`ParserRolesOrUsersSet`), whose
/// lists end at `ends`.
pub(crate) fn role_set(
    p: &mut Parser<'_>,
    allow: Allow,
    ends: Stop<'_>,
) -> Option<CompletedMarker> {
    let m = p.start();
    role_list(p, allow, &|p| ends(p) || p.at_kw(K::Except))?;
    if p.eat_kw(K::Except) {
        let except = Allow {
            all: false,
            any: false,
            ..allow
        };
        role_list(p, except, ends)?;
    }
    Some(m.complete(p, SyntaxKind::RoleSet))
}

/// A [`RoleList`](SyntaxKind::RoleList).
fn role_list(p: &mut Parser<'_>, allow: Allow, ends: Stop<'_>) -> Option<CompletedMarker> {
    let m = p.start();
    let shape = fallback::comma_separated(p, ends, &mut |p| role(p, allow).map(drop));
    if shape.items == 0 {
        m.abandon(p);
        return p.expected("a user or role");
    }
    Some(m.complete(p, SyntaxKind::RoleList))
}

/// One item of a [`RoleList`](SyntaxKind::RoleList), keywords first, as
/// `parseBeforeExcept` tries them.
fn role(p: &mut Parser<'_>, allow: Allow) -> Option<CompletedMarker> {
    let keyword = match p.nth_keyword(0) {
        Some(K::None) => true,
        Some(K::All) => allow.all,
        Some(K::Any) => allow.any,
        Some(K::CurrentUser | K::Currentuser) => allow.current_user,
        _ => false,
    };
    if !keyword {
        return user_name(p);
    }
    let m = p.start();
    let current_user = matches!(p.nth_keyword(0), Some(K::CurrentUser | K::Currentuser));
    if p.at_kw(K::Currentuser) {
        p.bump();
    } else {
        p.bump_keyword();
    }
    if current_user && p.at(T::OpeningRoundBracket) && p.nth_at(1, T::ClosingRoundBracket) {
        p.bump();
        p.bump();
    }
    Some(m.complete(p, SyntaxKind::RoleKeyword))
}

/// A [`UserName`](SyntaxKind::UserName) (`ParserUserNameWithHost`).
fn user_name(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    let m = p.start();
    if p.at(T::StringLiteral) {
        exprs::string(p)?;
    } else {
        names::name(p)?;
        if p.eat(T::At) {
            name_or_string(p)?;
        }
    }
    Some(m.complete(p, SyntaxKind::UserName))
}

/// A name or a string (`parseIdentifierOrStringLiteral`).
fn name_or_string(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    if p.at(T::StringLiteral) {
        exprs::string(p)
    } else {
        names::name(p)
    }
}

/// Items separated by commas as a [`NameList`](SyntaxKind::NameList), with
/// items that end at `ends`.
fn name_list(
    p: &mut Parser<'_>,
    ends: Stop<'_>,
    item: fn(&mut Parser<'_>) -> Option<CompletedMarker>,
) -> Option<CompletedMarker> {
    let m = p.start();
    let shape = fallback::comma_separated(p, ends, &mut |p| item(p).map(drop));
    if shape.items == 0 {
        m.abandon(p);
        return p.expected("a name");
    }
    Some(m.complete(p, SyntaxKind::NameList))
}

/// The names of `entity` in a `CREATE`, `ALTER` or `DROP`, with items that
/// end at `ends`.
fn entity_names(p: &mut Parser<'_>, entity: Entity, ends: Stop<'_>) -> Option<CompletedMarker> {
    match entity {
        Entity::User | Entity::Role => name_list(p, ends, user_name),
        Entity::RowPolicy => row_policy_names(p),
        Entity::Quota | Entity::Profile => name_list(p, ends, name_or_string),
    }
}

/// A [`RowPolicyNameList`](SyntaxKind::RowPolicyNameList)
/// (`ParserRowPolicyNames`): groups of `names ON tables`, separated by
/// commas. After a comma, a table without a database before `ON` (but not
/// `ON CLUSTER`) starts the next group.
fn row_policy_names(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    let list = p.start();
    loop {
        let item = p.start();
        name_or_string(p)?;
        while p.at(T::Comma) && (p.nth_at(1, T::StringLiteral) || names::at_name(p, 1)) {
            p.bump();
            name_or_string(p)?;
        }
        cluster(p)?;
        p.expect_kw(K::On)?;
        policy_table(p)?;
        while p.at(T::Comma)
            && !(names::at_name(p, 1) && p.nth_at_kw(2, K::On) && !p.nth_at_kw(3, K::Cluster))
        {
            if !(names::at_name(p, 1) || p.nth_at(1, T::Asterisk)) {
                break;
            }
            p.bump();
            policy_table(p)?;
        }
        cluster(p)?;
        item.complete(p, SyntaxKind::RowPolicyName);
        if p.at(T::Comma) && (p.nth_at(1, T::StringLiteral) || names::at_name(p, 1)) {
            p.bump();
        } else {
            break;
        }
    }
    Some(list.complete(p, SyntaxKind::RowPolicyNameList))
}

/// The table of a row policy: `*`, `db.*`, `db.table` or `table`, as a
/// [`GrantTarget`](SyntaxKind::GrantTarget).
fn policy_table(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    let m = p.start();
    if !p.eat(T::Asterisk) {
        names::name(p)?;
        if p.eat(T::Dot) && !p.eat(T::Asterisk) {
            names::name(p)?;
        }
    }
    Some(m.complete(p, SyntaxKind::GrantTarget))
}

// ---- SET ROLE and DROP ------------------------------------------------------

/// A [`SetRoleQuery`](SyntaxKind::SetRoleQuery).
fn set_role(p: &mut Parser<'_>) -> Option<()> {
    let m = p.start();
    p.bump_keyword();
    let roles = Allow {
        all: true,
        ..Allow::default()
    };
    if p.eat_kw(K::Role) {
        if !p.eat_kw(K::Default) {
            role_set(p, roles, &fallback::never)?;
        }
    } else {
        p.expect_kw(K::Default)?;
        p.expect_kw(K::Role)?;
        cluster(p)?;
        role_set(p, roles, &|p| p.at_kw(K::To))?;
        p.expect_kw(K::To)?;
        let users = Allow {
            current_user: true,
            ..Allow::default()
        };
        role_set(p, users, &at_on_cluster)?;
        cluster(p)?;
    }
    m.complete(p, SyntaxKind::SetRoleQuery);
    Some(())
}

/// A [`DropQuery`](SyntaxKind::DropQuery) of an access entity: `DROP`, the
/// entity's keywords, any [`IfExists`](SyntaxKind::IfExists), the names,
/// `FROM` and a storage, and any `ON CLUSTER`.
fn drop_entity(p: &mut Parser<'_>) -> Option<()> {
    let m = p.start();
    p.bump_keyword();
    let (entity, words) = entity(p, 0)?;
    for _ in 0..words {
        p.bump_keyword();
    }
    common::if_exists(p);
    entity_names(p, entity, &|p| at_on_cluster(p) || p.at_kw(K::From))?;
    if p.eat_kw(K::From) {
        name_or_string(p)?;
    }
    cluster(p)?;
    m.complete(p, SyntaxKind::DropQuery);
    Some(())
}

// ---- CREATE and ALTER -------------------------------------------------------

/// An [`AccessEntityQuery`](SyntaxKind::AccessEntityQuery).
fn create_or_alter(p: &mut Parser<'_>) -> Option<()> {
    let m = p.start();
    let alter = p.at_kw(K::Alter);
    p.bump_keyword();
    let (entity, words) = entity(p, 0)?;
    for _ in 0..words {
        p.bump_keyword();
    }
    if alter {
        common::if_exists(p);
    } else if common::if_not_exists(p).is_none() {
        p.eat_kws(&[K::Or, K::Replace]);
    }
    let at_clause = move |p: &Parser<'_>| at_clause(p, entity, alter);
    entity_names(p, entity, &at_clause)?;
    while !p.at_end() {
        if at_on_cluster(p) {
            fallback::clause(p, &at_clause, |p| common::on_cluster(p).map(drop));
        } else if at_clause(p) {
            fallback::clause(p, &at_clause, |p| clause(p, entity, alter).map(drop));
        } else {
            return p.expected("a clause");
        }
    }
    m.complete(p, SyntaxKind::AccessEntityQuery);
    Some(())
}

/// Whether a clause of a `CREATE` or `ALTER` of `entity` starts here.
fn at_clause(p: &Parser<'_>, entity: Entity, alter: bool) -> bool {
    if at_on_cluster(p) || p.at_kw(K::In) || p.at_kws(&[K::Rename, K::To]) {
        return true;
    }
    if alter && matches!(p.nth_keyword(0), Some(K::Add | K::Drop | K::Modify)) {
        return true;
    }
    let settings = matches!(
        p.nth_keyword(0),
        Some(K::Settings | K::Setting | K::Profile | K::Profiles)
    );
    match entity {
        Entity::User => {
            settings
                || matches!(
                    p.nth_keyword(0),
                    Some(K::Identified | K::Host | K::Grantees | K::Valid)
                )
                || p.at_kws(&[K::Not, K::Identified])
                || p.at_kws(&[K::Reset, K::Authentication])
                || p.at_kws(&[K::Default, K::Role])
                || p.at_kws(&[K::Default, K::Database])
                || (!alter && p.at_kw(K::Role))
        }
        Entity::Role => settings,
        Entity::Profile => settings || p.at_kw(K::Inherit) || p.at_kw(K::To),
        Entity::Quota => {
            matches!(
                p.nth_keyword(0),
                Some(K::For | K::To | K::Ipv4PrefixBits | K::Ipv6PrefixBits)
            ) || p.at_kws(&[K::Keyed, K::By])
                || p.at_kws(&[K::Key, K::By])
                || p.at_kws(&[K::Not, K::Keyed])
        }
        Entity::RowPolicy => {
            matches!(p.nth_keyword(0), Some(K::As | K::For | K::Using | K::To))
                || p.at_kws(&[K::With, K::Check])
        }
    }
}

/// One clause of a `CREATE` or `ALTER` of `entity`, as an
/// [`AccessClause`](SyntaxKind::AccessClause).
fn clause(p: &mut Parser<'_>, entity: Entity, alter: bool) -> Option<CompletedMarker> {
    let at_next = move |p: &Parser<'_>| at_clause(p, entity, alter);
    let m = p.start();
    let all = Allow {
        all: true,
        any: false,
        current_user: true,
    };
    if p.eat_kw(K::In) {
        name_or_string(p)?;
    } else if p.eat_kws(&[K::Rename, K::To]) {
        if entity == Entity::User {
            user_name(p)?;
        } else {
            name_or_string(p)?;
        }
    } else if alter && matches!(p.nth_keyword(0), Some(K::Add | K::Drop | K::Modify)) {
        // What is added or dropped (`HOST …`, `ALL SETTINGS`, `IDENTIFIED
        // …`) holds clause keywords of its own, so it runs to the next
        // `ADD`, `DROP` or `MODIFY`.
        p.bump_keyword();
        let rest = p.start();
        p.bump();
        fallback::skip(p, &|p| {
            matches!(p.nth_keyword(0), Some(K::Add | K::Drop | K::Modify))
                || at_on_cluster(p)
                || p.at_kws(&[K::Rename, K::To])
        });
        rest.complete(p, SyntaxKind::Opaque);
    } else if matches!(
        p.nth_keyword(0),
        Some(K::Settings | K::Setting | K::Profile | K::Profiles | K::Inherit)
    ) {
        if matches!(p.nth_keyword(0), Some(K::Settings | K::Setting)) {
            p.bump_keyword();
        }
        profile_settings(p, entity, &at_next)?;
    } else {
        match entity {
            Entity::User => user_clause(p, alter, &at_next)?,
            Entity::Quota => quota_clause(p, all, &at_next)?,
            Entity::RowPolicy => policy_clause(p, all, &at_next)?,
            Entity::Role | Entity::Profile => {
                p.expect_kw(K::To)?;
                role_set(p, all, &at_next)?;
            }
        }
    }
    Some(m.complete(p, SyntaxKind::AccessClause))
}

/// The body of a clause of `CREATE USER` or `ALTER USER` other than the
/// shared ones, from its keywords.
fn user_clause(p: &mut Parser<'_>, alter: bool, at_next: Stop<'_>) -> Option<()> {
    let tail = |p: &mut Parser<'_>| {
        if p.at_end() || at_next(p) {
            Some(())
        } else {
            fallback::opaque_until(p, at_next).map(drop)
        }
    };
    if p.eat_kws(&[K::Not, K::Identified]) {
        return Some(());
    }
    if p.eat_kw(K::Identified) {
        let with = p.eat_kw(K::With);
        let list = p.start();
        let mut typed = with;
        while auth_method(p, typed, at_next)?
            && p.at(T::Comma)
            && ((p.nth_at_kw(1, K::By) && p.nth_at(2, T::StringLiteral))
                || (names::at_name(p, 1) && p.nth_at_kw(2, K::By)))
        {
            p.bump();
            typed = !(p.at_kw(K::By) && p.nth_at(1, T::StringLiteral));
        }
        list.complete(p, SyntaxKind::AuthMethodList);
        return tail(p);
    }
    if p.at_kws(&[K::Reset, K::Authentication]) {
        p.bump_keyword();
        p.bump_keyword();
        return tail(p);
    }
    if p.eat_kw(K::Host) {
        let list = p.start();
        let shape = fallback::comma_separated(p, at_next, &mut |p| host(p).map(drop));
        if shape.items == 0 {
            list.abandon(p);
            return p.expected("a host");
        }
        list.complete(p, SyntaxKind::HostList);
        return Some(());
    }
    if p.eat_kw(K::Valid) {
        if p.eat_kw(K::Until) && p.at(T::StringLiteral) {
            exprs::string(p)?;
        }
        return tail(p);
    }
    if p.eat_kws(&[K::Default, K::Database]) {
        if !p.eat_kw(K::None) {
            names::name(p)?;
        }
        return Some(());
    }
    if p.eat_kw(K::Grantees) {
        let allow = Allow {
            all: false,
            any: true,
            current_user: true,
        };
        return role_set(p, allow, at_next).map(drop);
    }
    let default = p.eat_kw(K::Default);
    if !alter || default {
        p.expect_kw(K::Role)?;
        let allow = Allow {
            all: default,
            ..Allow::default()
        };
        return role_set(p, allow, at_next).map(drop);
    }
    p.expected("a clause of USER")
}

/// An [`AuthMethod`](SyntaxKind::AuthMethod) (`parseAuthenticationData`),
/// with its type first if `typed`. Returns whether another method may
/// follow it after a comma: what a method of a rarer type holds after its
/// type is kept as written up to the next clause, commas included.
fn auth_method(p: &mut Parser<'_>, typed: bool, at_next: Stop<'_>) -> Option<bool> {
    if !typed && !p.at_kw(K::By) {
        return p.expected("`BY`");
    }
    let m = p.start();
    if typed {
        names::name(p)?;
    }
    let mut more = true;
    if p.eat_kw(K::By) && p.at(T::StringLiteral) {
        exprs::string(p)?;
        if p.at_kw(K::Salt) && p.nth_at(1, T::StringLiteral) {
            p.bump_keyword();
            exprs::string(p)?;
        }
    } else if !(p.at_end() || at_next(p) || p.at(T::Comma)) {
        fallback::opaque_until(p, at_next)?;
        more = false;
    }
    if p.at_kws(&[K::Valid, K::Until]) && p.nth_at(2, T::StringLiteral) {
        p.bump_keyword();
        p.bump_keyword();
        exprs::string(p)?;
    }
    m.complete(p, SyntaxKind::AuthMethod);
    Some(more)
}

/// A [`HostItem`](SyntaxKind::HostItem).
fn host(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    let m = p.start();
    match p.nth_keyword(0) {
        Some(K::Local | K::Any | K::None) => p.bump_keyword(),
        Some(K::Name | K::Regexp | K::Ip | K::Like) => {
            p.bump_keyword();
            exprs::string(p)?;
        }
        _ => {
            exprs::string(p)?;
        }
    }
    Some(m.complete(p, SyntaxKind::HostItem))
}

/// A [`ProfileSettingList`](SyntaxKind::ProfileSettingList)
/// (`parseSettingsProfileElements`), with items that end at `ends`.
fn profile_settings(p: &mut Parser<'_>, entity: Entity, ends: Stop<'_>) -> Option<CompletedMarker> {
    let m = p.start();
    let mut first = true;
    let shape = fallback::comma_separated(p, ends, &mut |p| {
        let prefix = !std::mem::take(&mut first);
        profile_setting(p, entity, prefix).map(drop)
    });
    if shape.items == 0 {
        m.abandon(p);
        return p.expected("a setting");
    }
    Some(m.complete(p, SyntaxKind::ProfileSettingList))
}

/// A [`ProfileSetting`](SyntaxKind::ProfileSetting). `PROFILE` and
/// `INHERIT` (in a settings profile) are keywords before anything else, as
/// `parse_element` tries them first. `SETTINGS` before an element is one
/// only after a comma (`prefix`): the clause's own `SETTINGS` is that of
/// the first element, so `SETTINGS setting = 1` sets `setting`.
fn profile_setting(p: &mut Parser<'_>, entity: Entity, prefix: bool) -> Option<CompletedMarker> {
    if prefix && matches!(p.nth_keyword(0), Some(K::Settings | K::Setting)) {
        p.bump_keyword();
    }
    let m = p.start();
    let profile = if p.eat_kw(K::Profiles) || p.eat_kw(K::Profile) {
        true
    } else if entity == Entity::Profile && p.eat_kw(K::Inherit) {
        if !p.eat_kw(K::Profiles) {
            p.eat_kw(K::Profile);
        }
        true
    } else {
        false
    };
    if p.eat_kw(K::None) {
        return Some(m.complete(p, SyntaxKind::ProfileSetting));
    }
    if profile || p.at(T::StringLiteral) {
        name_or_string(p)?;
        return Some(m.complete(p, SyntaxKind::ProfileSetting));
    }
    names::compound_name(p)?;
    loop {
        if p.eat(T::Equals) {
            exprs::expr(p)?;
        } else if p.at_kw(K::Min) || p.at_kw(K::Max) {
            p.bump_keyword();
            p.eat(T::Equals);
            exprs::expr(p)?;
        } else if matches!(
            p.nth_keyword(0),
            Some(K::Readonly | K::Const | K::Writable | K::ChangeableInReadonly)
        ) {
            p.bump_keyword();
        } else {
            break;
        }
    }
    Some(m.complete(p, SyntaxKind::ProfileSetting))
}

/// The body of a clause of a quota other than the shared ones.
fn quota_clause(p: &mut Parser<'_>, all: Allow, at_next: Stop<'_>) -> Option<()> {
    if p.eat_kws(&[K::Not, K::Keyed]) {
        return Some(());
    }
    if p.at_kw(K::Ipv4PrefixBits) || p.at_kw(K::Ipv6PrefixBits) {
        p.bump_keyword();
        if !p.at(T::Number) {
            return p.expected("a number");
        }
        return exprs::literal(p).map(drop);
    }
    if p.eat_kws(&[K::Keyed, K::By]) || p.eat_kws(&[K::Key, K::By]) {
        return name_list(p, at_next, name_or_string).map(drop);
    }
    if p.eat_kw(K::To) {
        return role_set(p, all, at_next).map(drop);
    }
    let list = p.start();
    loop {
        quota_interval(p)?;
        if p.at(T::Comma) && p.nth_at_kw(1, K::For) {
            p.bump();
        } else {
            break;
        }
    }
    list.complete(p, SyntaxKind::QuotaIntervalList);
    Some(())
}

/// A [`QuotaInterval`](SyntaxKind::QuotaInterval)
/// (`parseIntervalsWithLimits`).
fn quota_interval(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    let m = p.start();
    p.expect_kw(K::For)?;
    p.eat_kw(K::Randomized);
    p.eat_kw(K::Interval);
    if !p.at(T::Number) {
        return p.expected("a number");
    }
    exprs::literal(p)?;
    if !exprs::at_interval_unit(p, 0) {
        return p.expected("an interval unit");
    }
    let unit = p.start();
    p.bump_keyword();
    unit.complete(p, SyntaxKind::IntervalUnit);
    if !(p.eat_kws(&[K::No, K::Limits]) || p.eat_kws(&[K::Tracking, K::Only])) {
        let list = p.start();
        // `MAX` before the first limit stands for those after it too.
        let mut max = false;
        loop {
            max |= p.at_kw(K::Max);
            quota_limit(p, max)?;
            if p.at(T::Comma) && !p.nth_at_kw(1, K::For) {
                p.bump();
            } else {
                break;
            }
        }
        list.complete(p, SyntaxKind::QuotaLimitList);
    }
    Some(m.complete(p, SyntaxKind::QuotaInterval))
}

/// A [`QuotaLimit`](SyntaxKind::QuotaLimit) (`parseLimits`); `max` says
/// whether a `MAX` came before.
fn quota_limit(p: &mut Parser<'_>, max: bool) -> Option<CompletedMarker> {
    let m = p.start();
    p.eat_kw(K::Max);
    let mut n = 0;
    while p.nth_at(n, T::BareWord) && !p.nth_at_kw(n, K::Max) {
        n += 1;
    }
    if n == 0 {
        return p.expected("a quota resource");
    }
    for _ in 0..n {
        p.bump();
    }
    if max {
        p.eat(T::Equals);
    } else {
        p.expect_kw(K::Max)?;
    }
    if !(p.at(T::Number) || p.at(T::StringLiteral)) {
        return p.expected("a number or string");
    }
    exprs::literal(p)?;
    Some(m.complete(p, SyntaxKind::QuotaLimit))
}

/// The body of a clause of a row policy other than the shared ones.
fn policy_clause(p: &mut Parser<'_>, all: Allow, at_next: Stop<'_>) -> Option<()> {
    if p.eat_kw(K::As) {
        if !(p.eat_kw(K::Permissive) || p.eat_kw(K::Restrictive)) {
            return p.expected("PERMISSIVE or RESTRICTIVE");
        }
        return Some(());
    }
    if p.eat_kw(K::For) {
        loop {
            if !matches!(
                p.nth_keyword(0),
                Some(K::Select | K::Insert | K::Update | K::Delete | K::All)
            ) {
                return p.expected("a command");
            }
            p.bump_keyword();
            if p.at(T::Comma)
                && matches!(
                    p.nth_keyword(1),
                    Some(K::Select | K::Insert | K::Update | K::Delete | K::All)
                )
            {
                p.bump();
            } else {
                break;
            }
        }
        return Some(());
    }
    if p.eat_kw(K::To) {
        return role_set(p, all, at_next).map(drop);
    }
    if !(p.eat_kw(K::Using) || p.eat_kws(&[K::With, K::Check])) {
        return p.expected("a clause of ROW POLICY");
    }
    if !p.eat_kw(K::None) {
        exprs::expr(p)?;
    }
    Some(())
}

#[cfg(test)]
mod tests;
