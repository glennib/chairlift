//! The kinds of tokens and nodes in the syntax tree.

use crate::lexer::TokenKind;

/// What a kind is for, beyond being a token or a node.
///
/// The statistics (see [`StatementStats`](super::StatementStats)) and the
/// printer find lists, clauses and verbatim regions by their role, so a new
/// node kind only needs the right role to be counted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Role {
    /// A token: one of the lexer's, or a bare word the parser re-classified
    /// ([`Keyword`](SyntaxKind::Keyword), [`LiteralWord`](SyntaxKind::LiteralWord)).
    Token,
    /// A node with no role below.
    Node,
    /// A statement's body: the child of a [`Statement`](SyntaxKind::Statement)
    /// for a statement the parser knows.
    Statement,
    /// A clause of a statement, such as `WHERE …`.
    Clause,
    /// A list whose child nodes are its items, separated by commas.
    List,
    /// Text kept verbatim because it did not parse; see [`FallbackLevel`].
    Fallback(FallbackLevel),
    /// Text kept verbatim by design, such as a window specification: not a
    /// parse failure.
    Opaque,
}

/// How much of a statement a failed parse left verbatim, smallest first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FallbackLevel {
    /// One item of a list, up to the next `,` or the end of the list.
    Item,
    /// One clause, up to the next clause keyword.
    Clause,
    /// The whole statement.
    Statement,
}

macro_rules! syntax_kinds {
    (
        tokens { $( $(#[$tmeta:meta])* $tok:ident = $lex:ident, )* }
        words { $( $(#[$wmeta:meta])* $word:ident, )* }
        nodes { $( $(#[$nmeta:meta])* $node:ident: $role:ident, )* }
    ) => {
        /// The kind of a token or node of the syntax tree.
        ///
        /// Token kinds mirror the lexer's [`TokenKind`]s one to one (see
        /// [`SyntaxKind::from_token_kind`]), plus the bare words the parser
        /// re-classified: [`Keyword`](Self::Keyword) and
        /// [`LiteralWord`](Self::LiteralWord). Every other kind is a node.
        ///
        /// The node kinds are in sections: structure, names, expressions, data
        /// types, shared clause parts, then one section per statement grammar,
        /// whose kinds may be declared before the grammar uses them.
        /// [`role`](Self::role) says what each kind is for.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        #[repr(u16)]
        pub enum SyntaxKind {
            $( $(#[$tmeta])* $tok, )*
            $( $(#[$wmeta])* $word, )*
            $( $(#[$nmeta])* $node, )*
        }

        impl SyntaxKind {
            /// Every kind, in declaration order.
            pub const ALL: &[SyntaxKind] = &[
                $( SyntaxKind::$tok, )*
                $( SyntaxKind::$word, )*
                $( SyntaxKind::$node, )*
            ];

            /// The syntax kind of a lexer token.
            #[must_use]
            pub const fn from_token_kind(kind: TokenKind) -> SyntaxKind {
                match kind {
                    $( TokenKind::$lex => SyntaxKind::$tok, )*
                }
            }

            /// What the kind is for.
            #[must_use]
            pub const fn role(self) -> Role {
                match self {
                    $( SyntaxKind::$tok => Role::Token, )*
                    $( SyntaxKind::$word => Role::Token, )*
                    $( SyntaxKind::$node => role!($role), )*
                }
            }

            /// The kind's name, as in the source: `BinaryExpr`.
            #[must_use]
            pub const fn name(self) -> &'static str {
                match self {
                    $( SyntaxKind::$tok => stringify!($tok), )*
                    $( SyntaxKind::$word => stringify!($word), )*
                    $( SyntaxKind::$node => stringify!($node), )*
                }
            }
        }
    };
}

macro_rules! role {
    (Node) => {
        Role::Node
    };
    (Statement) => {
        Role::Statement
    };
    (Clause) => {
        Role::Clause
    };
    (List) => {
        Role::List
    };
    (Opaque) => {
        Role::Opaque
    };
    (FallbackItem) => {
        Role::Fallback(FallbackLevel::Item)
    };
    (FallbackClause) => {
        Role::Fallback(FallbackLevel::Clause)
    };
    (FallbackStatement) => {
        Role::Fallback(FallbackLevel::Statement)
    };
}

syntax_kinds! {
    tokens {
        /// A run of whitespace.
        Whitespace = Whitespace,
        /// `-- …`, `// …`, `# …`.
        LineComment = LineComment,
        /// `/* … */`.
        BlockComment = BlockComment,
        /// A bare word used as a name, or one the parser did not classify.
        BareWord = BareWord,
        /// A number.
        Number = Number,
        /// A string literal.
        StringLiteral = StringLiteral,
        /// `` `…` `` or `"…"`.
        QuotedIdentifier = QuotedIdentifier,
        /// `(`
        OpeningRoundBracket = OpeningRoundBracket,
        /// `)`
        ClosingRoundBracket = ClosingRoundBracket,
        /// `[`
        OpeningSquareBracket = OpeningSquareBracket,
        /// `]`
        ClosingSquareBracket = ClosingSquareBracket,
        /// `{`
        OpeningCurlyBrace = OpeningCurlyBrace,
        /// `}`
        ClosingCurlyBrace = ClosingCurlyBrace,
        /// `,`
        Comma = Comma,
        /// `;`
        Semicolon = Semicolon,
        /// `\G`
        VerticalDelimiter = VerticalDelimiter,
        /// `.`
        Dot = Dot,
        /// `*`
        Asterisk = Asterisk,
        /// `$tag$…$tag$`
        HereDoc = HereDoc,
        /// `$`
        DollarSign = DollarSign,
        /// `+`
        Plus = Plus,
        /// `-`
        Minus = Minus,
        /// `/`
        Slash = Slash,
        /// `%`
        Percent = Percent,
        /// `->`
        Arrow = Arrow,
        /// `?`
        QuestionMark = QuestionMark,
        /// `:`
        Colon = Colon,
        /// `^`
        Caret = Caret,
        /// `::`
        DoubleColon = DoubleColon,
        /// `=` or `==`
        Equals = Equals,
        /// `!=` or `<>`
        NotEquals = NotEquals,
        /// `<`
        Less = Less,
        /// `>`
        Greater = Greater,
        /// `<=`
        LessOrEquals = LessOrEquals,
        /// `>=`
        GreaterOrEquals = GreaterOrEquals,
        /// `<=>`
        Spaceship = Spaceship,
        /// `|`
        PipeMark = PipeMark,
        /// `|>`
        PipeOperator = PipeOperator,
        /// `||`
        Concatenation = Concatenation,
        /// `~`
        Tilde = Tilde,
        /// `~*`
        TildeAsterisk = TildeAsterisk,
        /// `!~`
        NotTilde = NotTilde,
        /// `!~*`
        NotTildeAsterisk = NotTildeAsterisk,
        /// `@`
        At = At,
        /// `@@`
        DoubleAt = DoubleAt,
        /// `{{ … }}`
        JinjaExpr = JinjaExpr,
        /// `{% … %}`, which the parser steps over like a comment.
        JinjaBlock = JinjaBlock,
        /// `{# … #}`, which the parser steps over like a comment.
        JinjaComment = JinjaComment,
        /// `{% raw %}…{% endraw %}` (the lexer's
        /// [`Verbatim`](TokenKind::Verbatim)).
        JinjaRaw = Verbatim,
        /// The inline data of an `INSERT`.
        InsertData = InsertData,
        /// An unrecognised character.
        Error = Error,
        /// An unclosed `/*`.
        ErrorMultilineCommentIsNotClosed = ErrorMultilineCommentIsNotClosed,
        /// An unclosed `'`.
        ErrorSingleQuoteIsNotClosed = ErrorSingleQuoteIsNotClosed,
        /// An unclosed `"`.
        ErrorDoubleQuoteIsNotClosed = ErrorDoubleQuoteIsNotClosed,
        /// An unclosed `` ` ``.
        ErrorBackQuoteIsNotClosed = ErrorBackQuoteIsNotClosed,
        /// A lone `!`.
        ErrorSingleExclamationMark = ErrorSingleExclamationMark,
        /// A malformed number.
        ErrorWrongNumber = ErrorWrongNumber,
        /// An unclosed Jinja tag.
        ErrorJinjaTagIsNotClosed = ErrorJinjaTagIsNotClosed,
        /// An unclosed `{% raw %}`.
        ErrorJinjaRawIsNotClosed = ErrorJinjaRawIsNotClosed,
    }
    words {
        /// A bare word the parser consumed in a keyword slot: a statement or
        /// clause keyword, an operator word, a column-definition keyword, or
        /// `NULL`. Its text is one of ClickHouse's keyword words, and its
        /// canonical case is upper case.
        Keyword,
        /// A literal written as a word whose canonical case is lower case:
        /// `true`, `false`, `inf`, `infinity` and `nan`, also where `true`
        /// and `false` stand in a keyword slot (`x IS TRUE`).
        LiteralWord,
    }
    nodes {
        // ---- Structure -------------------------------------------------
        /// The whole text: statements, and the trivia between them.
        Root: Node,
        /// One statement: its body, if it has one, then the `;` that ends it,
        /// if there is one.
        Statement: Node,
        /// A statement kept verbatim: one of a kind the parser does not know,
        /// or one that did not parse.
        VerbatimStatement: FallbackStatement,
        /// A clause kept verbatim because it did not parse.
        VerbatimClause: FallbackClause,
        /// A list item kept verbatim because it did not parse.
        VerbatimItem: FallbackItem,
        /// Text kept verbatim by design, such as the window specification of
        /// `OVER (…)`.
        Opaque: Opaque,
        /// The body of a `SELECT <list>` statement as the expression probe
        /// parses it (see [`Entry::SelectListProbe`](super::Entry)): `SELECT`
        /// and an [`ExprList`](Self::ExprList). Never produced for files.
        ExpressionProbe: Statement,

        // ---- Names -----------------------------------------------------
        /// One identifier: a bare word, a quoted identifier, a Jinja
        /// expression, or a `{name:Identifier}` parameter; for a data type,
        /// its name, which may be several words (`DOUBLE PRECISION`).
        Name: Node,
        /// A dotted name, `db.table` or `a.b.c`: [`Name`](Self::Name)s
        /// separated by `.`.
        QualifiedName: Node,
        /// An alias: `AS` and a name, or a name alone.
        Alias: Node,

        // ---- Expressions -----------------------------------------------
        /// A literal: a number, string, heredoc, `NULL`, `true`, `false`,
        /// `inf` or `nan`.
        Literal: Node,
        /// `DATE '…'` or `TIMESTAMP '…'`.
        TypedLiteral: Node,
        /// A literal whose text ClickHouse reads as written, so its bytes,
        /// whitespace included, must be kept: the left operand of `::` and
        /// the first argument of `CAST`, when they are a number, string,
        /// `NULL`, or an array or tuple of those. ClickHouse composes the
        /// text from the tokens and keeps whether whitespace follows each
        /// comma. Nothing inside it is a keyword slot.
        LiteralAsText: Node,
        /// `( expr )`.
        ParenExpr: Node,
        /// `( a, b )`, `()`, `( a, )`.
        TupleExpr: List,
        /// `[ a, b ]`, or `ARRAY[ a, b ]`.
        ArrayExpr: List,
        /// `{ 'key': value, … }`: a map with string keys, as the value of a
        /// setting (string values) or the cursor of `STREAM CURSOR`
        /// (numbers or nested maps).
        MapExpr: List,
        /// `'key': value` in a [`MapExpr`](Self::MapExpr).
        MapEntry: Node,
        /// Operands and the binary operators between them, for every binary
        /// operator but `AND` and `OR`: one node for a run of operators of
        /// one priority, which group from the left (`a - b + c` is
        /// `(a - b) + c`). An operator may be several tokens (`NOT LIKE`,
        /// `IS NOT DISTINCT FROM`, `AT TIME ZONE`), and `LIKE` and its
        /// relatives may have `ESCAPE 'c'` after their right operand.
        BinaryExpr: Node,
        /// `a AND b AND c`: one node for a run of `AND`s.
        AndExpr: Node,
        /// `a OR b OR c`: one node for a run of `OR`s.
        OrExpr: Node,
        /// `-x`, `+x`, `NOT x`.
        PrefixExpr: Node,
        /// `x IS [NOT] NULL`, `x IS [NOT] TRUE`, `x AT LOCAL`: an operand
        /// and the operator words after it.
        PostfixExpr: Node,
        /// `x [NOT] BETWEEN a AND b`.
        BetweenExpr: Node,
        /// `c ? a : b`.
        TernaryExpr: Node,
        /// `x -> body`, `(x, y) -> body`.
        LambdaExpr: Node,
        /// `x::Type`.
        CastExpr: Node,
        /// `a[i]`.
        IndexExpr: Node,
        /// `x.1`, `f(x).name`, `x.-1`: tuple element access on an expression
        /// that is not a plain name. The element is a number token, a
        /// [`Name`](Self::Name), or an expression.
        TupleAccessExpr: Node,
        /// `ANY (subquery)`, `SOME (…)`, `ALL (…)` as the right operand of a
        /// comparison or a matching predicate: the keyword, then a
        /// [`Subquery`](Self::Subquery) or a [`ParenExpr`](Self::ParenExpr).
        QuantifiedExpr: Node,
        /// `INTERVAL n UNIT`, `INTERVAL '…' [UNIT [TO UNIT]]`.
        IntervalExpr: Node,
        /// An interval unit: `DAY`, `HOUR`, …, in `INTERVAL` and as the first
        /// argument of `dateAdd` and its relatives.
        IntervalUnit: Node,
        /// `CASE [x] WHEN … THEN … [ELSE …] END`.
        CaseExpr: Node,
        /// `WHEN a THEN b` in a [`CaseExpr`](Self::CaseExpr).
        CaseWhen: Node,
        /// `ELSE c` in a [`CaseExpr`](Self::CaseExpr).
        CaseElse: Node,
        /// `{name:Type}`.
        QueryParameter: Node,
        /// `@@name`.
        GlobalVariable: Node,
        /// An expression with an [`Alias`](Self::Alias).
        AliasedExpr: Node,
        /// A function call: its name, an [`ArgList`](Self::ArgList) or
        /// [`KeywordArgList`](Self::KeywordArgList), a second `ArgList` for a
        /// parametric function (`quantile(0.5)(x)`), then any of
        /// [`FunctionFilter`](Self::FunctionFilter),
        /// [`NullsModifier`](Self::NullsModifier) and [`Over`](Self::Over).
        /// `exists(SELECT …)` has a [`Subquery`](Self::Subquery) in place of
        /// the arguments.
        FunctionCall: Node,
        /// `( [DISTINCT | ALL] a, b )`: comma-separated arguments.
        ArgList: List,
        /// The arguments of a function that takes keywords between them:
        /// `CAST(x AS T)`, `EXTRACT(DAY FROM d)`, `SUBSTRING(s FROM 1 FOR 2)`,
        /// `TRIM(BOTH ' ' FROM s)`, `POSITION(a IN b)`,
        /// `OVERLAY(s PLACING r FROM 1)`.
        KeywordArgList: Node,
        /// `FILTER (WHERE …)` after a function call.
        FunctionFilter: Node,
        /// `RESPECT NULLS` or `IGNORE NULLS` after a function call.
        NullsModifier: Node,
        /// `OVER name` or `OVER (…)`, with the window specification
        /// [`Opaque`](Self::Opaque).
        Over: Node,
        /// `*` or `t.*`, with any `LIKE '…'` and column transformers.
        AsteriskExpr: Node,
        /// `COLUMNS(…)` or `t.COLUMNS(…)`: `COLUMNS`, a
        /// [`ColumnList`](Self::ColumnList), then any column transformers.
        ColumnsMatcher: Node,
        /// `( a, t.b )` or `( '…' )`: the names or the pattern of a columns
        /// matcher, as `COLUMNS(…)` and `EXCEPT (…)` take them, or the
        /// replacements of `REPLACE (…)`.
        ColumnList: List,
        /// `APPLY …`, `EXCEPT [STRICT] …` or `REPLACE [STRICT] …` after
        /// `*` or `COLUMNS(…)`.
        ColumnTransformer: Node,
        /// A comma-separated list of expressions.
        ExprList: List,

        // ---- Data types ------------------------------------------------
        /// A data type: its [`Name`](Self::Name), then any
        /// [`TypeArgList`](Self::TypeArgList).
        DataType: Node,
        /// `( … )` after a data type's name.
        TypeArgList: List,
        /// `name Type`, an element of `Tuple(…)`, `Nested(…)` or `JSON(…)`.
        NameTypePair: Node,
        /// `SKIP path` or `SKIP REGEXP '…'` in `JSON(…)`.
        JsonSkip: Node,

        // ---- Shared clause parts ---------------------------------------
        /// An `ORDER BY` item: an expression with any `AS` alias, then any of
        /// `ASC`/`DESC`, `NULLS FIRST|LAST`, `COLLATE '…'`, `WITH FILL [FROM
        /// …] [TO …] [STEP …] [STALENESS …]`; in a table's `ORDER BY`, only a
        /// direction.
        OrderByItem: Node,
        /// The items of `ORDER BY`.
        OrderByList: List,
        /// `SETTINGS a = 1, …`.
        SettingsClause: Clause,
        /// The items of `SETTINGS`.
        SettingsList: List,
        /// `name = value` in a [`SettingsList`](Self::SettingsList), or
        /// `name` alone, which ClickHouse reads as `name = true`.
        SettingItem: Node,
        /// `ON CLUSTER name`.
        OnCluster: Node,
        /// `IF NOT EXISTS`.
        IfNotExists: Node,
        /// `IF EXISTS`.
        IfExists: Node,

        // ---- SELECT ----------------------------------------------------
        // Grammar in `grammar/select.rs`. A clause node holds its keywords as
        // tokens, then its body: an expression, or a list node whose child
        // nodes are the items.
        //
        // Output clauses: `INTO OUTFILE`, `FORMAT`, and a `SETTINGS` after
        // either, are the statement's, as ClickHouse attaches them
        // (`ParserQueryWithOutput`): they are the last children of the
        // statement's body, a `SelectQuery` that is the whole statement, a
        // `UnionQuery` (set operations, or a query in parentheses, wherever
        // the parentheses are), or a `CreateTable` or
        // `CreateMaterializedView` after its `AsClause`. A `SETTINGS` right
        // after a query without parentheses, before any output clause, is
        // that query's own clause. A query inside another, or after `AS`,
        // has no output clauses.
        /// One query: its clauses in the order written, which may be any
        /// order, with repeats, since each clause is parsed (or kept
        /// verbatim) on its own: [`WithClause`](Self::WithClause),
        /// [`SelectClause`](Self::SelectClause),
        /// [`FromClause`](Self::FromClause), then any
        /// [`JoinClause`](Self::JoinClause) and
        /// [`ArrayJoinClause`](Self::ArrayJoinClause),
        /// [`PrewhereClause`](Self::PrewhereClause),
        /// [`WhereClause`](Self::WhereClause),
        /// [`GroupByClause`](Self::GroupByClause),
        /// [`HavingClause`](Self::HavingClause),
        /// [`WindowClause`](Self::WindowClause),
        /// [`QualifyClause`](Self::QualifyClause),
        /// [`OrderByClause`](Self::OrderByClause),
        /// [`InterpolateClause`](Self::InterpolateClause),
        /// [`LimitByClause`](Self::LimitByClause),
        /// [`LimitClause`](Self::LimitClause),
        /// [`OffsetClause`](Self::OffsetClause),
        /// [`FetchClause`](Self::FetchClause),
        /// [`SettingsClause`](Self::SettingsClause), and
        /// [`VerbatimClause`](Self::VerbatimClause) for one that did not
        /// parse; then, if the query is the whole statement, its output
        /// clauses ([`IntoOutfileClause`](Self::IntoOutfileClause),
        /// [`FormatClause`](Self::FormatClause), `SettingsClause`). A query
        /// may start with `FROM` (`FROM t SELECT x`, or `FROM t` alone for
        /// `SELECT *`), and always has a `SelectClause` or a `FromClause`,
        /// parsed or verbatim: a `WITH` clause alone is no query, and a
        /// statement of one is verbatim.
        SelectQuery: Statement,
        /// Queries joined by `UNION`, `EXCEPT` or `INTERSECT`, or one query
        /// in parentheses: operands ([`SelectQuery`](Self::SelectQuery) or
        /// [`Subquery`](Self::Subquery)) separated by
        /// [`SetOperator`](Self::SetOperator)s, as one flat run whatever the
        /// operators' priorities; then, if it is the whole statement, its
        /// output clauses ([`IntoOutfileClause`](Self::IntoOutfileClause),
        /// [`FormatClause`](Self::FormatClause),
        /// [`SettingsClause`](Self::SettingsClause)). Nothing else follows
        /// an operand in parentheses.
        UnionQuery: Statement,
        /// `UNION`, `EXCEPT` or `INTERSECT`, with any `ALL` or `DISTINCT`.
        SetOperator: Node,
        /// `( query )`: in an expression, in `FROM`, in a CTE, or as an
        /// operand of a set operation. It holds a
        /// [`SelectQuery`](Self::SelectQuery) or
        /// [`UnionQuery`](Self::UnionQuery), [`ValuesRows`](Self::ValuesRows),
        /// or an `EXPLAIN …` kept [`Opaque`](Self::Opaque).
        Subquery: Node,
        /// `VALUES (…), (…)` in parentheses, SQL's table of rows: `VALUES`
        /// and an [`ExprList`](Self::ExprList) of the rows.
        ValuesRows: Node,
        /// `WITH [RECURSIVE]` and a [`WithList`](Self::WithList).
        WithClause: Clause,
        /// The items of `WITH`: [`CommonTableExpr`](Self::CommonTableExpr)s,
        /// and expressions, each with any `AS` alias
        /// ([`AliasedExpr`](Self::AliasedExpr)).
        WithList: List,
        /// `name [(columns)] AS [MATERIALIZED] (query)` in `WITH`: a
        /// [`Name`](Self::Name), any [`ParenList`](Self::ParenList) of
        /// column names, and a [`Subquery`](Self::Subquery).
        CommonTableExpr: Node,
        /// `SELECT`, any of `ALL`, `DISTINCT` or
        /// [`DistinctOn`](Self::DistinctOn) and a
        /// [`TopClause`](Self::TopClause), then a
        /// [`SelectList`](Self::SelectList).
        SelectClause: Clause,
        /// `DISTINCT ON` and a [`ParenList`](Self::ParenList).
        DistinctOn: Node,
        /// `TOP n` or `TOP (n)`, with any `WITH TIES`.
        TopClause: Node,
        /// The items of `SELECT`: expressions, each with any alias
        /// ([`AliasedExpr`](Self::AliasedExpr)). A trailing comma is kept in
        /// the list.
        SelectList: List,
        /// `FROM` and a [`TableList`](Self::TableList).
        FromClause: Clause,
        /// The tables of `FROM`, [`TableExpr`](Self::TableExpr)s separated by
        /// commas (comma joins).
        TableList: List,
        /// A table in `FROM` or a join: a [`Name`](Self::Name) or
        /// [`QualifiedName`](Self::QualifiedName), a table function (a
        /// [`FunctionCall`](Self::FunctionCall)), a
        /// [`Subquery`](Self::Subquery), a file name as a string
        /// [`Literal`](Self::Literal), or a [`ParenJoin`](Self::ParenJoin),
        /// in an [`AliasedExpr`](Self::AliasedExpr) if it has an alias; then
        /// any [`ParenList`](Self::ParenList) of column aliases, `FINAL`,
        /// [`SampleClause`](Self::SampleClause),
        /// [`StreamClause`](Self::StreamClause), and a `ParenList` of the
        /// query's column aliases (`FROM t FINAL (a, b)`). A query as the
        /// argument of `view(…)`, `eval(…)` or `obfuscate(…)` is the only
        /// child node of the call's [`ArgList`](Self::ArgList); that of
        /// `viewIfPermitted(…)` holds the query, `ELSE` and the
        /// `FunctionCall` that follows it, with no comma; `exists(…)` has a
        /// [`Subquery`](Self::Subquery) in place of the `ArgList`, as in an
        /// expression.
        TableExpr: Node,
        /// `( tables )`: a [`TableList`](Self::TableList) and joins in
        /// parentheses ([`JoinClause`](Self::JoinClause)s and
        /// [`ArrayJoinClause`](Self::ArrayJoinClause)s), standing for one
        /// table. Its tables have no alias without `AS`.
        ParenJoin: Node,
        /// `SAMPLE k [OFFSET m]` after a table; `k` and `m` are expressions
        /// (`1 / 10`).
        SampleClause: Node,
        /// `STREAM [BOUNDED] [UNORDERED] [CURSOR {…}] [WATERMARK FOR c AS e
        /// [IDLE TIMEOUT INTERVAL n unit]]` after a table.
        StreamClause: Node,
        /// A join: the join's keywords as tokens (`GLOBAL ANY LEFT OUTER
        /// JOIN`, `CROSS JOIN`, …), any `LATERAL`, a
        /// [`TableExpr`](Self::TableExpr), and a [`JoinOn`](Self::JoinOn) or
        /// [`JoinUsing`](Self::JoinUsing) if it has a condition. A comma join
        /// that follows a join (`… JOIN b ON x, c`) is a join too: `,` and a
        /// `TableExpr`.
        JoinClause: Clause,
        /// `ON` and the join's condition.
        JoinOn: Node,
        /// `USING` and a [`ParenList`](Self::ParenList), or an
        /// [`ExprList`](Self::ExprList) without parentheses.
        JoinUsing: Node,
        /// `[LEFT | INNER] ARRAY JOIN` and an [`ExprList`](Self::ExprList).
        ArrayJoinClause: Clause,
        /// `PREWHERE` and an expression.
        PrewhereClause: Clause,
        /// `WHERE` and an expression.
        WhereClause: Clause,
        /// `GROUP BY` and `ALL`, `ROLLUP` or `CUBE` and a
        /// [`ParenList`](Self::ParenList), a
        /// [`GroupingSets`](Self::GroupingSets), or an
        /// [`ExprList`](Self::ExprList); then any
        /// [`GroupByModifier`](Self::GroupByModifier)s. ClickHouse allows the
        /// modifiers without `GROUP BY` (`SELECT count() FROM t WITH
        /// TOTALS`), and the clause is then only those. In a projection it
        /// is `GROUP BY` and an `ExprList`.
        GroupByClause: Clause,
        /// `WITH ROLLUP`, `WITH CUBE` or `WITH TOTALS`.
        GroupByModifier: Node,
        /// `GROUPING SETS ( … )`: its items are
        /// [`ParenList`](Self::ParenList)s and expressions.
        GroupingSets: List,
        /// `HAVING` and an expression.
        HavingClause: Clause,
        /// `WINDOW` and a [`WindowList`](Self::WindowList).
        WindowClause: Clause,
        /// The items of `WINDOW`.
        WindowList: List,
        /// `name AS (…)` in `WINDOW`, with the definition
        /// [`Opaque`](Self::Opaque).
        WindowDefinition: Node,
        /// `QUALIFY` and an expression.
        QualifyClause: Clause,
        /// `ORDER BY …`. In a query, an [`OrderByList`](Self::OrderByList),
        /// or `ALL` with any direction and `NULLS FIRST|LAST` as tokens; in a
        /// projection, an `OrderByList`. In a table definition
        /// (`ParserStorageOrderByClause`), one expression (a
        /// [`TupleExpr`](Self::TupleExpr) for `(a, b)`, a
        /// [`FunctionCall`](Self::FunctionCall) for `tuple()`), an
        /// [`OrderByItem`](Self::OrderByItem) of an expression and a
        /// direction, or an [`OrderByTuple`](Self::OrderByTuple) when the
        /// keys in brackets have directions.
        OrderByClause: Clause,
        /// `INTERPOLATE`, after `ORDER BY … WITH FILL`, with any
        /// [`ParenList`](Self::ParenList) of
        /// [`InterpolateItem`](Self::InterpolateItem)s.
        InterpolateClause: Clause,
        /// `column [AS expr]` in `INTERPOLATE`.
        InterpolateItem: Node,
        /// `LIMIT n [OFFSET m] BY …` or `LIMIT m, n BY …`, with an
        /// [`ExprList`](Self::ExprList) or `ALL` after `BY`.
        LimitByClause: Clause,
        /// `LIMIT n`, `LIMIT m, n` or `LIMIT n OFFSET m`, with any `WITH
        /// TIES`, and `LIMIT [n] AFTER a [ALL] [UNTIL b]` or `LIMIT UNTIL b`:
        /// the keywords as tokens between expressions.
        LimitClause: Clause,
        /// `OFFSET n [ROW | ROWS]`.
        OffsetClause: Clause,
        /// `FETCH FIRST|NEXT n ROW|ROWS ONLY|WITH TIES`.
        FetchClause: Clause,
        /// `FORMAT` and a [`Name`](Self::Name): an output clause of a query
        /// statement or of `CREATE` (see above).
        FormatClause: Clause,
        /// `INTO OUTFILE`, the file name as a string
        /// [`Literal`](Self::Literal), then any of `APPEND`, `TRUNCATE` and
        /// `AND STDOUT` as tokens, and `COMPRESSION` with a string `Literal`
        /// and any `LEVEL` with a number `Literal`; of a query or a `CREATE`
        /// statement.
        IntoOutfileClause: Clause,
        /// `( a, b )`: a list in parentheses that is not an expression, after
        /// `DISTINCT ON`, `USING`, `ROLLUP`, `CUBE` and `INTERPOLATE`, in
        /// `GROUPING SETS`, the column names after a CTE's name, and a
        /// table's column aliases.
        ParenList: List,

        // ---- CREATE ----------------------------------------------------
        // Grammar in `grammar/create.rs`. A statement is its
        // `CreateHeader`, then for a table its `TableElementList`, then its
        // clauses, each a child of the statement in the order written.
        /// `CREATE [OR REPLACE] [TEMPORARY] TABLE …`: a
        /// [`CreateHeader`](Self::CreateHeader), an optional
        /// [`TableElementList`](Self::TableElementList), then clauses,
        /// among them the statement's output clauses: a
        /// [`FormatClause`](Self::FormatClause) or
        /// [`IntoOutfileClause`](Self::IntoOutfileClause) after the
        /// [`AsClause`](Self::AsClause)'s query is the statement's, as is a
        /// [`SettingsClause`](Self::SettingsClause) after one of those.
        CreateTable: Statement,
        /// `CREATE [OR REPLACE] MATERIALIZED VIEW …`: a
        /// [`CreateHeader`](Self::CreateHeader), then clauses
        /// ([`RefreshClause`](Self::RefreshClause), [`ToClause`](Self::ToClause),
        /// storage clauses, [`PopulateClause`](Self::PopulateClause), …), with
        /// the view's [`TableElementList`](Self::TableElementList) where it is
        /// written, and the query in an [`AsClause`](Self::AsClause); output
        /// clauses after the query are the statement's, as in
        /// [`CreateTable`](Self::CreateTable).
        CreateMaterializedView: Statement,
        /// The statement's keywords up to the object kind (`CREATE OR
        /// REPLACE TABLE`), an [`IfNotExists`](Self::IfNotExists), the name
        /// (a [`Name`](Self::Name) or [`QualifiedName`](Self::QualifiedName)),
        /// any [`TableUuid`](Self::TableUuid)s and an
        /// [`OnCluster`](Self::OnCluster).
        CreateHeader: Node,
        /// `UUID '…'` after a table's name, or `TO INNER UUID '…'`.
        TableUuid: Node,
        /// `( columns, indexes, … )` of `CREATE TABLE` or a view: `(`, the
        /// elements ([`ColumnDef`](Self::ColumnDef), [`IndexDef`](Self::IndexDef),
        /// [`ProjectionDef`](Self::ProjectionDef),
        /// [`ConstraintDef`](Self::ConstraintDef),
        /// [`TablePrimaryKey`](Self::TablePrimaryKey),
        /// [`ForeignKeyDef`](Self::ForeignKeyDef)) separated by commas, a
        /// trailing comma if written, `)`.
        TableElementList: List,
        /// A column definition: the column's [`Name`](Self::Name), its
        /// [`DataType`](Self::DataType) unless a default stands in for it, then
        /// one node per modifier in the order written: [`ColumnCollate`](Self::ColumnCollate),
        /// [`ColumnNull`](Self::ColumnNull), [`ColumnDefault`](Self::ColumnDefault),
        /// [`ColumnComment`](Self::ColumnComment), [`ColumnCodec`](Self::ColumnCodec),
        /// [`ColumnStatistics`](Self::ColumnStatistics), [`ColumnTtl`](Self::ColumnTtl),
        /// [`ColumnPrimaryKey`](Self::ColumnPrimaryKey) and
        /// [`ColumnSettings`](Self::ColumnSettings).
        ColumnDef: Node,
        /// `COLLATE name` of a column.
        ColumnCollate: Node,
        /// `NULL` or `NOT NULL` of a column.
        ColumnNull: Node,
        /// `DEFAULT expr`, `MATERIALIZED expr`, `ALIAS expr`, `EPHEMERAL
        /// [expr]` or `AUTO_INCREMENT`.
        ColumnDefault: Node,
        /// `CODEC` and a [`CodecList`](Self::CodecList), of a column or after
        /// `RECOMPRESS` in a TTL rule.
        ColumnCodec: Node,
        /// `STATISTICS` and a [`CodecList`](Self::CodecList) of statistics
        /// kinds.
        ColumnStatistics: Node,
        /// `( … )` after `CODEC` or `STATISTICS`: names, each a
        /// [`Name`](Self::Name) or, with arguments, a
        /// [`FunctionCall`](Self::FunctionCall) of a `Name` and an
        /// [`ArgList`](Self::ArgList). The names keep their case.
        CodecList: List,
        /// `TTL expr` of a column.
        ColumnTtl: Node,
        /// `COMMENT '…'` of a column.
        ColumnComment: Node,
        /// `SETTINGS (` a [`SettingsList`](Self::SettingsList) `)` of a column.
        ColumnSettings: Node,
        /// `PRIMARY KEY` of a column.
        ColumnPrimaryKey: Node,
        /// `INDEX name expr TYPE type [GRANULARITY n]`, where the type is a
        /// [`Name`](Self::Name) or a [`FunctionCall`](Self::FunctionCall) of a
        /// `Name` and an [`ArgList`](Self::ArgList).
        IndexDef: Node,
        /// `PROJECTION name`, then a [`Subquery`](Self::Subquery) or `INDEX`,
        /// an [`ExprList`](Self::ExprList), `TYPE` and a type (as for
        /// [`IndexDef`](Self::IndexDef)), then any
        /// [`ProjectionSettings`](Self::ProjectionSettings).
        ProjectionDef: Node,
        /// `WITH SETTINGS (` a [`SettingsList`](Self::SettingsList) `)` of a
        /// projection.
        ProjectionSettings: Node,
        /// `CONSTRAINT name CHECK|ASSUME expr`.
        ConstraintDef: Node,
        /// `PRIMARY KEY expr` inside the element list.
        TablePrimaryKey: Node,
        /// `FOREIGN KEY expr REFERENCES table expr [ON DELETE|UPDATE action]…`,
        /// which ClickHouse reads and ignores.
        ForeignKeyDef: Node,
        /// `ENGINE [=] name[(args)]`: the engine is a [`Name`](Self::Name),
        /// or a [`FunctionCall`](Self::FunctionCall) of a `Name` and an
        /// [`ArgList`](Self::ArgList); its name keeps its case (`Null`).
        EngineClause: Clause,
        /// `PARTITION BY expr`.
        PartitionByClause: Clause,
        /// `PRIMARY KEY expr`.
        PrimaryKeyClause: Clause,
        /// `UNIQUE KEY expr`.
        UniqueKeyClause: Clause,
        /// `SAMPLE BY expr`.
        SampleByClause: Clause,
        /// `( … )` or `tuple( … )` of a table's `ORDER BY` whose keys have a
        /// direction: an optional [`Name`](Self::Name) `tuple`, `(`, an
        /// [`OrderByList`](Self::OrderByList), `)`. Without directions the
        /// keys are an ordinary expression, a [`TupleExpr`](Self::TupleExpr)
        /// or a [`FunctionCall`](Self::FunctionCall).
        OrderByTuple: Node,
        /// `TTL` and a [`TtlList`](Self::TtlList), of a table.
        TtlClause: Clause,
        /// The rules of a table's `TTL`, [`TtlElement`](Self::TtlElement)s
        /// separated by commas.
        TtlList: List,
        /// One rule of a table's `TTL`: an expression, then its action as
        /// keywords and nodes: `[DELETE] [WHERE expr]`, `TO DISK|VOLUME`, any
        /// [`IfExists`](Self::IfExists) and a string
        /// [`Literal`](Self::Literal), `RECOMPRESS` and a
        /// [`ColumnCodec`](Self::ColumnCodec), or `GROUP BY` an
        /// [`ExprList`](Self::ExprList) with any `SET` and an
        /// [`AssignmentList`](Self::AssignmentList).
        TtlElement: Node,
        /// [`Assignment`](Self::Assignment)s separated by commas.
        AssignmentList: List,
        /// `column = expr`.
        Assignment: Node,
        /// `COMMENT '…'` of a table or view.
        CommentClause: Clause,
        /// `AS` and a query (a [`SelectQuery`](Self::SelectQuery) or
        /// [`UnionQuery`](Self::UnionQuery), without output clauses), a
        /// [`FunctionCall`](Self::FunctionCall) of a table function, or a
        /// table's name (a [`Name`](Self::Name) or
        /// [`QualifiedName`](Self::QualifiedName)).
        AsClause: Clause,
        /// `TO` and a [`Name`](Self::Name) or
        /// [`QualifiedName`](Self::QualifiedName) of a materialized view's
        /// target, or `TO INNER UUID` and a string [`Literal`](Self::Literal).
        ToClause: Clause,
        /// `POPULATE`, `EMPTY` or `CLONE`: whether a materialized view starts
        /// with the data its query selects, or a table created `AS …` with
        /// none or a copy.
        PopulateClause: Clause,
        /// `DEFINER = user` and `SQL SECURITY DEFINER|INVOKER|NONE`, either or
        /// both.
        SqlSecurityClause: Clause,
        /// `REFRESH EVERY|AFTER interval [OFFSET interval] [RANDOMIZE FOR
        /// interval]`, each interval a [`TimeInterval`](Self::TimeInterval),
        /// then any of a [`DependsOn`](Self::DependsOn), a
        /// [`SettingsClause`](Self::SettingsClause) and `APPEND
        /// [INCREMENTAL]`, of a materialized view; `REFRESH DEPENDS ON …`
        /// may leave out the schedule.
        RefreshClause: Clause,
        /// `n UNIT [n UNIT …]` in a [`RefreshClause`](Self::RefreshClause):
        /// numbers and [`IntervalUnit`](Self::IntervalUnit)s.
        TimeInterval: Node,
        /// `DEPENDS ON` and table names ([`Name`](Self::Name)s or
        /// [`QualifiedName`](Self::QualifiedName)s) separated by commas.
        DependsOn: List,
        /// A target table of a `TimeSeries` table: `SAMPLES`, `DATA`,
        /// `RECENT SAMPLES`, `TAGS`, `TIME RANGES`, `METRIC FAMILIES` or
        /// `METRICS`, then `INNER UUID '…'`, `INNER COLUMNS` and a
        /// [`TableElementList`](Self::TableElementList), `[INNER]` and the
        /// target's storage clauses, or the target's name.
        ViewTargetClause: Clause,

        // ---- INSERT ----------------------------------------------------
        // Grammar in `grammar/insert.rs`. The data after `VALUES` or
        // `FORMAT name` is one `InsertData` token, kept byte for byte.
        /// `[WITH …] INSERT INTO …`: any [`WithClause`](Self::WithClause),
        /// an [`InsertInto`](Self::InsertInto), then in the order
        /// ClickHouse reads them any [`InfileClause`](Self::InfileClause)
        /// and [`SettingsClause`](Self::SettingsClause), and an
        /// [`InsertValues`](Self::InsertValues), an
        /// [`InsertFormat`](Self::InsertFormat), or a query (a
        /// [`SelectQuery`](Self::SelectQuery) or
        /// [`UnionQuery`](Self::UnionQuery), without output clauses) with
        /// any `InsertFormat` after it. A `SETTINGS` at the end of the query
        /// is the query's own clause.
        InsertQuery: Statement,
        /// `INSERT INTO [TABLE]` and the target, a table's
        /// [`Name`](Self::Name) or [`QualifiedName`](Self::QualifiedName),
        /// or `FUNCTION` and a table function's
        /// [`FunctionCall`](Self::FunctionCall) with any
        /// [`PartitionByClause`](Self::PartitionByClause); then any
        /// [`InsertColumns`](Self::InsertColumns).
        InsertInto: Node,
        /// `( a, t.b, * EXCEPT (c), t.*, COLUMNS('x') )`: the columns an
        /// `INSERT` fills, names ([`Name`](Self::Name),
        /// [`QualifiedName`](Self::QualifiedName)),
        /// [`AsteriskExpr`](Self::AsteriskExpr)s and
        /// [`ColumnsMatcher`](Self::ColumnsMatcher)s, with any trailing
        /// comma.
        InsertColumns: List,
        /// `FROM INFILE` and the file name as a string
        /// [`Literal`](Self::Literal), then any `COMPRESSION` and a string
        /// `Literal`.
        InfileClause: Clause,
        /// `VALUES` and the rows as one [`InsertData`](Self::InsertData)
        /// token, which is missing when `;` or the end follows `VALUES`.
        InsertValues: Clause,
        /// `FORMAT` and the format's [`Name`](Self::Name), then the data as
        /// one [`InsertData`](Self::InsertData) token if any follows.
        InsertFormat: Clause,

        // ---- Statements of a line or two ----------------------------------
        // Grammar in `grammar/simple.rs`. A statement holds its keywords as
        // tokens and its names, lists and clauses as nodes, in the order
        // written. Those ClickHouse reads with `ParserQueryWithOutput` end
        // with any output clauses (`IntoOutfileClause`, `FormatClause`,
        // `SettingsClause`).
        /// `DROP`, `DETACH`, `TRUNCATE` or `UNDROP`, and the object's
        /// keywords (`[TEMPORARY] TABLE`, `VIEW`, `DICTIONARY`, `DATABASE`,
        /// `FUNCTION`, `NAMED COLLECTION`, `INDEX`, `WORKLOAD`,
        /// `RESOURCE`, `HANDLER`, `[ALL] TABLES FROM`) as tokens; any
        /// [`IfExists`](Self::IfExists) and `IF EMPTY`; the name, a
        /// [`Name`](Self::Name) or [`QualifiedName`](Self::QualifiedName),
        /// or for `DROP` of tables a [`NameList`](Self::NameList); `ON` and
        /// the table of `DROP INDEX`; `[NOT] LIKE|ILIKE` and a string
        /// [`Literal`](Self::Literal) after `TABLES FROM`; a
        /// [`TableUuid`](Self::TableUuid) after `UNDROP`'s name; then any
        /// [`OnCluster`](Self::OnCluster), `PERMANENTLY`, `SYNC` or `NO
        /// DELAY`, and output clauses.
        DropQuery: Statement,
        /// `ATTACH` and `[TEMPORARY] TABLE`, `[MATERIALIZED] VIEW`,
        /// `DICTIONARY` or `DATABASE`, any
        /// [`IfNotExists`](Self::IfNotExists), the name, any
        /// [`TableUuid`](Self::TableUuid), `FROM` and a string
        /// [`Literal`](Self::Literal) or `AS [NOT] REPLICATED`, any
        /// [`OnCluster`](Self::OnCluster), and the definition that may follow,
        /// [`Opaque`](Self::Opaque).
        AttachQuery: Statement,
        /// `RENAME [TABLE | DICTIONARY | DATABASE]` or `EXCHANGE TABLES |
        /// DICTIONARIES`, a [`RenameList`](Self::RenameList), any
        /// [`OnCluster`](Self::OnCluster), then output clauses.
        RenameQuery: Statement,
        /// [`RenamePair`](Self::RenamePair)s separated by commas.
        RenameList: List,
        /// `[IF EXISTS] a TO b`, or `a AND b` to exchange: an
        /// [`IfExists`](Self::IfExists), and two names, each a
        /// [`Name`](Self::Name) or [`QualifiedName`](Self::QualifiedName).
        RenamePair: Node,
        /// `OPTIMIZE TABLE` and a name, then in this order any of an
        /// [`OnCluster`](Self::OnCluster), a [`Partition`](Self::Partition),
        /// `DRY RUN PARTS` and an [`ExprList`](Self::ExprList) of strings,
        /// `FINAL` or `FORCE`, `DEDUPLICATE`, `CLEANUP`, `MANIFEST`, and `BY`
        /// and an `ExprList` of names,
        /// [`AsteriskExpr`](Self::AsteriskExpr)s and
        /// [`ColumnsMatcher`](Self::ColumnsMatcher)s; then output clauses.
        OptimizeQuery: Statement,
        /// `PARTITION` and `ID` with a string [`Literal`](Self::Literal) or a
        /// [`QueryParameter`](Self::QueryParameter), `ALL`, or an
        /// expression.
        Partition: Node,
        /// `SET` and a [`SettingsList`](Self::SettingsList), or `SET TIME
        /// ZONE`, any `=`, and a [`Literal`](Self::Literal) or a
        /// [`MapExpr`](Self::MapExpr).
        SetQuery: Statement,
        /// `USE`, any `DATABASE`, and a [`Name`](Self::Name).
        UseQuery: Statement,
        /// Names ([`Name`](Self::Name)s or
        /// [`QualifiedName`](Self::QualifiedName)s) separated by commas.
        NameList: List,

        // ---- ALTER and the lightweight mutations ---------------------------
        // Grammar in `grammar/alter.rs`. A command holds its keywords as
        // tokens and its names, definitions, lists and clauses as nodes, in
        // the order written.
        /// `ALTER [TEMPORARY] TABLE` or `ALTER DATABASE`, the name (a
        /// [`Name`](Self::Name) or [`QualifiedName`](Self::QualifiedName)),
        /// any [`OnCluster`](Self::OnCluster), an
        /// [`AlterCommandList`](Self::AlterCommandList), then output clauses.
        AlterQuery: Statement,
        /// [`AlterCommand`](Self::AlterCommand)s separated by commas.
        AlterCommandList: List,
        /// One command of `ALTER`, `ADD COLUMN …` or `DROP PARTITION …`: its
        /// keywords, then in the order written its names, an
        /// [`IfExists`](Self::IfExists) or [`IfNotExists`](Self::IfNotExists),
        /// a definition ([`ColumnDef`](Self::ColumnDef),
        /// [`IndexDef`](Self::IndexDef), [`ProjectionDef`](Self::ProjectionDef),
        /// [`ConstraintDef`](Self::ConstraintDef),
        /// [`StatisticsDef`](Self::StatisticsDef)), a
        /// [`Partition`](Self::Partition) or [`InPartition`](Self::InPartition),
        /// a list ([`SettingsList`](Self::SettingsList),
        /// [`TtlList`](Self::TtlList), [`AssignmentList`](Self::AssignmentList),
        /// [`NameList`](Self::NameList)), a [`WhereClause`](Self::WhereClause),
        /// a query, or other clauses. In brackets if written so.
        AlterCommand: Node,
        /// `IN PARTITION` and one partition, or for a mutation several
        /// separated by commas: each `ID` and a string, `ALL`, or an
        /// expression.
        InPartition: Node,
        /// The columns of `ADD`, `MODIFY`, `DROP`, `CLEAR` or `MATERIALIZE
        /// STATISTICS`, a [`NameList`](Self::NameList), then for the first two
        /// `TYPE` and an [`ExprList`](Self::ExprList) of the statistics
        /// kinds, each a [`Name`](Self::Name) or a
        /// [`FunctionCall`](Self::FunctionCall).
        StatisticsDef: Node,
        /// `DELETE FROM` a table, any [`OnCluster`](Self::OnCluster) and
        /// [`InPartition`](Self::InPartition), a
        /// [`WhereClause`](Self::WhereClause) and any
        /// [`SettingsClause`](Self::SettingsClause): the lightweight delete.
        DeleteQuery: Statement,
        /// `UPDATE` a table, any [`OnCluster`](Self::OnCluster), an
        /// [`UpdateSetClause`](Self::UpdateSetClause), any
        /// [`InPartition`](Self::InPartition), a
        /// [`WhereClause`](Self::WhereClause) and any
        /// [`SettingsClause`](Self::SettingsClause): the lightweight update.
        UpdateQuery: Statement,
        /// `SET` and an [`AssignmentList`](Self::AssignmentList) of `UPDATE`.
        UpdateSetClause: Clause,

        // ---- Other CREATE kinds -------------------------------------------
        // Grammar in `grammar/create.rs` for views and databases and in
        // `grammar/create_other.rs` for dictionaries and functions.
        /// `CREATE [OR REPLACE] [TEMPORARY] VIEW …`: a
        /// [`CreateHeader`](Self::CreateHeader), any
        /// [`TableElementList`](Self::TableElementList), then clauses
        /// ([`SqlSecurityClause`](Self::SqlSecurityClause),
        /// [`CommentClause`](Self::CommentClause)) and the query in an
        /// [`AsClause`](Self::AsClause).
        CreateView: Statement,
        /// `CREATE DATABASE …`: a [`CreateHeader`](Self::CreateHeader),
        /// then clauses: [`EngineClause`](Self::EngineClause),
        /// [`SettingsClause`](Self::SettingsClause) and
        /// [`CommentClause`](Self::CommentClause).
        CreateDatabase: Statement,
        /// `CREATE [OR REPLACE] DICTIONARY …`: a
        /// [`CreateHeader`](Self::CreateHeader), a
        /// [`DictionaryAttributeList`](Self::DictionaryAttributeList), then
        /// clauses in the order written:
        /// [`DictionaryPrimaryKey`](Self::DictionaryPrimaryKey) and the
        /// [`DictionaryClause`](Self::DictionaryClause)s `SOURCE`, `LAYOUT`,
        /// `LIFETIME`, `RANGE` and `SETTINGS`, and
        /// [`CommentClause`](Self::CommentClause).
        CreateDictionary: Statement,
        /// `( attributes )` of a dictionary: `(`,
        /// [`DictionaryAttribute`](Self::DictionaryAttribute)s separated by
        /// commas, a trailing comma if written, `)`.
        DictionaryAttributeList: List,
        /// A dictionary attribute: its [`Name`](Self::Name), its
        /// [`DataType`](Self::DataType), then in any order `DEFAULT` and a
        /// literal, `EXPRESSION` and an expression, `HIERARCHICAL`,
        /// `BIDIRECTIONAL`, `INJECTIVE` and `IS_OBJECT_ID`.
        DictionaryAttribute: Node,
        /// `PRIMARY KEY` and the key attributes, a
        /// [`NameList`](Self::NameList), in brackets if written so.
        DictionaryPrimaryKey: Clause,
        /// `SOURCE`, `LAYOUT`, `LIFETIME`, `RANGE` or `SETTINGS`, then `(`, what
        /// it holds, `)`: a [`KeyValueFunction`](Self::KeyValueFunction) for
        /// `SOURCE` and `LAYOUT`, [`KeyValuePairs`](Self::KeyValuePairs) or a
        /// number for `LIFETIME` and `RANGE`, a
        /// [`SettingsList`](Self::SettingsList) for `SETTINGS`.
        DictionaryClause: Clause,
        /// `name(pairs)` in `SOURCE` or `LAYOUT`, `CLICKHOUSE(TABLE 'x')`: a
        /// [`Name`](Self::Name), then `(`, [`KeyValuePairs`](Self::KeyValuePairs)
        /// and `)`. The name is a name, never a keyword, and keeps its case.
        KeyValueFunction: Node,
        /// [`KeyValuePair`](Self::KeyValuePair)s, one after another without
        /// commas.
        KeyValuePairs: Node,
        /// A key, a [`Name`](Self::Name) that keeps its case whatever it
        /// spells (`TABLE`, `MIN`), and a value: a literal, a name, a
        /// function call, or `(`, [`KeyValuePairs`](Self::KeyValuePairs) and
        /// `)`.
        KeyValuePair: Node,
        /// `CREATE [OR REPLACE] FUNCTION [IF NOT EXISTS]`, the function's
        /// [`Name`](Self::Name), any [`OnCluster`](Self::OnCluster), `AS` and
        /// an expression, usually a [`LambdaExpr`](Self::LambdaExpr); or with
        /// what follows the name kept [`Opaque`](Self::Opaque) for the
        /// `ENGINE` and `LANGUAGE` forms.
        CreateFunction: Statement,

        // ---- Access control -----------------------------------------------
        // Grammar in `grammar/access.rs`. `DROP` of an access entity is a
        // `DropQuery` whose names are a `NameList` of `UserName`s, `Name`s
        // and string `Literal`s, or a `RowPolicyNameList`.
        /// `GRANT` or `REVOKE`: the keyword, any
        /// [`OnCluster`](Self::OnCluster), `GRANT OPTION FOR` or `ADMIN
        /// OPTION FOR` after `REVOKE`, what is granted (an
        /// [`AccessList`](Self::AccessList), a
        /// [`CurrentGrants`](Self::CurrentGrants) or a
        /// [`RoleSet`](Self::RoleSet) of roles), a
        /// [`GranteesClause`](Self::GranteesClause), and any
        /// [`GrantOptionClause`](Self::GrantOptionClause)s, with an
        /// `OnCluster` at any of the places ClickHouse takes one.
        GrantQuery: Statement,
        /// [`AccessElement`](Self::AccessElement)s separated by commas.
        AccessList: List,
        /// Privileges on one object: a
        /// [`PrivilegeList`](Self::PrivilegeList) and a
        /// [`GrantOn`](Self::GrantOn).
        AccessElement: Node,
        /// [`Privilege`](Self::Privilege)s separated by commas.
        PrivilegeList: List,
        /// One privilege: its words (`SELECT`, `ALTER UPDATE`, `dictGet`),
        /// then any [`ParenList`](Self::ParenList) of columns. The words are
        /// keywords when every one of them is a keyword word, and names
        /// otherwise, since ClickHouse matches them as one phrase.
        Privilege: Node,
        /// `ON` and the [`GrantTarget`](Self::GrantTarget) of privileges.
        GrantOn: Clause,
        /// What privileges or a row policy apply to: `*`, `*.*`, `db.*`,
        /// `db.table`, `table`, or a parameter (`S3`), each name with any
        /// `*` after it.
        GrantTarget: Node,
        /// `CURRENT GRANTS` and an [`AccessList`](Self::AccessList) in
        /// brackets, or a [`GrantOn`](Self::GrantOn).
        CurrentGrants: Node,
        /// `TO` or `FROM` and a [`RoleSet`](Self::RoleSet): who is granted.
        GranteesClause: Clause,
        /// `WITH GRANT OPTION`, `WITH ADMIN OPTION` or `WITH REPLACE OPTION`.
        GrantOptionClause: Clause,
        /// Users and roles: a [`RoleList`](Self::RoleList), then any `EXCEPT`
        /// and another `RoleList`.
        RoleSet: Node,
        /// [`UserName`](Self::UserName)s and
        /// [`RoleKeyword`](Self::RoleKeyword)s separated by commas.
        RoleList: List,
        /// `NONE`, `ALL`, `ANY` or `CURRENT_USER` (with any `()`) in a
        /// [`RoleList`](Self::RoleList). `currentUser` keeps its case.
        RoleKeyword: Node,
        /// A user or role: a [`Name`](Self::Name), a
        /// [`QueryParameter`](Self::QueryParameter) or a string
        /// [`Literal`](Self::Literal); a name may have `@` and a host
        /// pattern after it, a `Name` or a string `Literal`.
        UserName: Node,
        /// `CREATE` or `ALTER` of a `USER`, `ROLE`, `QUOTA`, `[ROW] POLICY` or
        /// `[SETTINGS] PROFILE`: the keywords; any
        /// [`IfNotExists`](Self::IfNotExists), `OR REPLACE` or
        /// [`IfExists`](Self::IfExists); the names, a
        /// [`NameList`](Self::NameList) or a
        /// [`RowPolicyNameList`](Self::RowPolicyNameList); then its
        /// [`AccessClause`](Self::AccessClause)s and
        /// [`OnCluster`](Self::OnCluster)s in the order written, which may be
        /// any order.
        AccessEntityQuery: Statement,
        /// A clause of an access entity: its keywords, then its body, such as
        /// a [`RoleSet`](Self::RoleSet), a
        /// [`ProfileSettingList`](Self::ProfileSettingList), a
        /// [`HostList`](Self::HostList), a
        /// [`QuotaIntervalList`](Self::QuotaIntervalList), a name, a string
        /// or an expression. What chairlift does not model in a clause
        /// (`IDENTIFIED WITH ssh_key BY KEY …`, `ADD HOST …` in `ALTER USER`)
        /// is [`Opaque`](Self::Opaque).
        AccessClause: Clause,
        /// [`HostItem`](Self::HostItem)s separated by commas.
        HostList: List,
        /// A host of `HOST`: `LOCAL`, `ANY` or `NONE`, `NAME`, `REGEXP`, `IP`
        /// or `LIKE` and a string [`Literal`](Self::Literal), or a string
        /// alone.
        HostItem: Node,
        /// [`ProfileSetting`](Self::ProfileSetting)s separated by commas.
        ProfileSettingList: List,
        /// One element of a settings profile: a setting's
        /// [`Name`](Self::Name) or [`QualifiedName`](Self::QualifiedName) and
        /// any of `= value`, `MIN [=] value`, `MAX [=] value` and `CONST`,
        /// `READONLY`, `WRITABLE` or `CHANGEABLE_IN_READONLY`; `PROFILE`,
        /// `PROFILES` or `INHERIT` and a profile's name; a profile's name
        /// alone; or `NONE`. Any `SETTINGS` before it is a token of its own.
        ProfileSetting: Node,
        /// [`QuotaInterval`](Self::QuotaInterval)s separated by commas.
        QuotaIntervalList: List,
        /// `FOR [RANDOMIZED] [INTERVAL] n` and an
        /// [`IntervalUnit`](Self::IntervalUnit), then `NO LIMITS`, `TRACKING
        /// ONLY` or a [`QuotaLimitList`](Self::QuotaLimitList).
        QuotaInterval: Node,
        /// [`QuotaLimit`](Self::QuotaLimit)s separated by commas.
        QuotaLimitList: List,
        /// A quota's limit: `MAX`, the resource's words, which keep their case,
        /// any `=`, and a number or string [`Literal`](Self::Literal); or the
        /// words, `MAX` and the value.
        QuotaLimit: Node,
        /// [`RowPolicyName`](Self::RowPolicyName)s separated by commas.
        RowPolicyNameList: List,
        /// Row policies: their names ([`Name`](Self::Name)s or string
        /// [`Literal`](Self::Literal)s separated by commas), any
        /// [`OnCluster`](Self::OnCluster), `ON` and the tables
        /// ([`GrantTarget`](Self::GrantTarget)s separated by commas), and any
        /// `OnCluster`.
        RowPolicyName: Node,
        /// `SET ROLE` and a [`RoleSet`](Self::RoleSet) or `DEFAULT`; or `SET
        /// DEFAULT ROLE`, any [`OnCluster`](Self::OnCluster), a `RoleSet`,
        /// `TO`, a `RoleSet` of users, and any `OnCluster`.
        SetRoleQuery: Statement,

        // ---- SHOW and DESCRIBE ----------------------------------------------
        // Grammar in `grammar/show.rs`.
        /// `SHOW` and the keywords of what it shows, then what the form
        /// takes: a name, a [`NameList`](Self::NameList), `FROM` or `IN` and
        /// a name, `[NOT] LIKE|ILIKE` and a string
        /// [`Literal`](Self::Literal), `LIMIT` and an expression, `FOR` and a
        /// [`RoleSet`](Self::RoleSet); what a form takes that chairlift does
        /// not model is [`Opaque`](Self::Opaque); then output clauses.
        ShowQuery: Statement,
        /// `DESCRIBE` or `DESC`, any `TABLE`, and a table: a
        /// [`Name`](Self::Name), [`QualifiedName`](Self::QualifiedName),
        /// table function ([`FunctionCall`](Self::FunctionCall)) or
        /// [`Subquery`](Self::Subquery); then output clauses.
        DescribeQuery: Statement,
    }
}

impl SyntaxKind {
    /// Whether the kind is a token.
    #[must_use]
    pub const fn is_token(self) -> bool {
        matches!(self.role(), Role::Token)
    }

    /// Whether the kind is trivia: whitespace, a comment, or a Jinja block or
    /// comment tag. Trivia never starts or ends a node; see
    /// [`syntax`](super#trivia).
    #[must_use]
    pub const fn is_trivia(self) -> bool {
        matches!(
            self,
            SyntaxKind::Whitespace
                | SyntaxKind::LineComment
                | SyntaxKind::BlockComment
                | SyntaxKind::JinjaBlock
                | SyntaxKind::JinjaComment
        )
    }

    /// Whether the kind is a list, whose child nodes are its items.
    #[must_use]
    pub const fn is_list(self) -> bool {
        matches!(self.role(), Role::List)
    }

    /// Whether the kind is a clause, parsed or verbatim.
    #[must_use]
    pub const fn is_clause(self) -> bool {
        matches!(
            self.role(),
            Role::Clause | Role::Fallback(FallbackLevel::Clause)
        )
    }

    /// Whether the kind's text is printed as written: a verbatim region,
    /// either a fallback or opaque by design.
    #[must_use]
    pub const fn is_verbatim(self) -> bool {
        matches!(self.role(), Role::Fallback(_) | Role::Opaque)
    }

    /// The fallback level, for a verbatim region that is a parse failure.
    #[must_use]
    pub const fn fallback_level(self) -> Option<FallbackLevel> {
        match self.role() {
            Role::Fallback(level) => Some(level),
            _ => None,
        }
    }
}

impl From<TokenKind> for SyntaxKind {
    fn from(kind: TokenKind) -> Self {
        SyntaxKind::from_token_kind(kind)
    }
}

impl From<SyntaxKind> for rowan::SyntaxKind {
    fn from(kind: SyntaxKind) -> Self {
        rowan::SyntaxKind(kind as u16)
    }
}

/// The [`rowan::Language`] of chairlift's syntax trees.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ClickHouse {}

impl rowan::Language for ClickHouse {
    type Kind = SyntaxKind;

    fn kind_from_raw(raw: rowan::SyntaxKind) -> SyntaxKind {
        // Trees are only built from `SyntaxKind`s, so every raw kind is one.
        SyntaxKind::ALL
            .get(usize::from(raw.0))
            .copied()
            .unwrap_or(SyntaxKind::Error)
    }

    fn kind_to_raw(kind: SyntaxKind) -> rowan::SyntaxKind {
        kind.into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discriminants_index_all() {
        for (i, kind) in SyntaxKind::ALL.iter().enumerate() {
            assert_eq!(*kind as usize, i, "{kind:?}");
        }
    }

    #[test]
    fn tokens_come_first_and_map_back() {
        let tokens = SyntaxKind::ALL.iter().take_while(|k| k.is_token()).count();
        assert!(SyntaxKind::ALL[tokens..].iter().all(|k| !k.is_token()));
        assert_eq!(
            SyntaxKind::from_token_kind(TokenKind::Verbatim),
            SyntaxKind::JinjaRaw
        );
        for kind in SyntaxKind::ALL.iter().filter(|k| k.is_trivia()) {
            assert!(kind.is_token());
        }
    }
}
