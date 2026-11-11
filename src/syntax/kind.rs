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
        /// An `ORDER BY` item: an expression, then any of `ASC`/`DESC`,
        /// `NULLS FIRST|LAST`, `COLLATE '…'`, `WITH FILL [FROM …] [TO …]
        /// [STEP …] [STALENESS …]`.
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
        /// [`SettingsClause`](Self::SettingsClause),
        /// [`FormatClause`](Self::FormatClause),
        /// [`IntoOutfileClause`](Self::IntoOutfileClause), and
        /// [`VerbatimClause`](Self::VerbatimClause) for one that did not
        /// parse. A query may start with `FROM` (`FROM t SELECT x`, or
        /// `FROM t` alone for `SELECT *`), and always has a `SelectClause`
        /// or a `FromClause`, parsed or verbatim: a `WITH` clause alone is
        /// no query, and a statement of one is verbatim.
        SelectQuery: Statement,
        /// Queries joined by `UNION`, `EXCEPT` or `INTERSECT`, or one query
        /// in parentheses: operands ([`SelectQuery`](Self::SelectQuery) or
        /// [`Subquery`](Self::Subquery)) separated by
        /// [`SetOperator`](Self::SetOperator)s, as one flat run whatever the
        /// operators' priorities. Clauses that follow an operand in
        /// parentheses, such as the `FORMAT` of `(SELECT 1) UNION ALL
        /// (SELECT 2) FORMAT JSON`, come after it here; after an operand
        /// without parentheses they are that operand's own.
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
        /// and expressions with an `AS` alias
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
        /// [`SampleClause`](Self::SampleClause) and
        /// [`StreamClause`](Self::StreamClause). A query as the argument of
        /// `view(…)` is the only item of the call's
        /// [`ArgList`](Self::ArgList).
        TableExpr: Node,
        /// `( tables )`: a [`TableList`](Self::TableList) and joins in
        /// parentheses, standing for one table.
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
        /// `GROUP BY` and one of `ALL`, `ROLLUP` or `CUBE` with a
        /// [`ParenList`](Self::ParenList),
        /// [`GroupingSets`](Self::GroupingSets), or an
        /// [`ExprList`](Self::ExprList); then any
        /// [`GroupByModifier`](Self::GroupByModifier)s. ClickHouse allows the
        /// modifiers without `GROUP BY` (`SELECT count() FROM t WITH
        /// TOTALS`), and the clause is then only those.
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
        /// `ORDER BY …`, in a query or a table definition: an
        /// [`OrderByList`](Self::OrderByList), or `ALL` with any direction
        /// and `NULLS FIRST|LAST` as tokens.
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
        /// `FORMAT name`.
        FormatClause: Clause,
        /// `INTO OUTFILE`, then the file name and its options
        /// [`Opaque`](Self::Opaque).
        IntoOutfileClause: Clause,
        /// `( a, b )`: a list in parentheses that is not an expression, after
        /// `DISTINCT ON`, `USING`, `ROLLUP`, `CUBE` and `INTERPOLATE`, in
        /// `GROUPING SETS`, and the column names after a CTE's name or a
        /// table's alias.
        ParenList: List,

        // ---- CREATE ----------------------------------------------------
        // Grammar in `grammar/create.rs`. A statement is its
        // `CreateHeader`, then for a table its `TableElementList`, then its
        // clauses, each a child of the statement in the order written.
        /// `CREATE [OR REPLACE] [TEMPORARY] TABLE …`: a
        /// [`CreateHeader`](Self::CreateHeader), an optional
        /// [`TableElementList`](Self::TableElementList), then clauses.
        CreateTable: Statement,
        /// `CREATE [OR REPLACE] MATERIALIZED VIEW …`: a
        /// [`CreateHeader`](Self::CreateHeader), then clauses
        /// ([`RefreshClause`](Self::RefreshClause), [`ToClause`](Self::ToClause),
        /// storage clauses, [`PopulateClause`](Self::PopulateClause), …), with
        /// the view's [`TableElementList`](Self::TableElementList) where it is
        /// written, and the query in an [`AsClause`](Self::AsClause).
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
        /// `PROJECTION name`, then a [`Subquery`](Self::Subquery) or `INDEX
        /// exprs TYPE type`, then any [`ProjectionSettings`](Self::ProjectionSettings).
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
        /// keywords and nodes: `[DELETE] [WHERE expr]`, `TO DISK|VOLUME [IF
        /// EXISTS] '…'`, `RECOMPRESS` and a [`ColumnCodec`](Self::ColumnCodec),
        /// or `GROUP BY` an [`ExprList`](Self::ExprList) with `SET` and an
        /// [`AssignmentList`](Self::AssignmentList).
        TtlElement: Node,
        /// [`Assignment`](Self::Assignment)s separated by commas.
        AssignmentList: List,
        /// `column = expr`.
        Assignment: Node,
        /// `COMMENT '…'` of a table or view.
        CommentClause: Clause,
        /// `AS` and a query, a [`FunctionCall`](Self::FunctionCall) of a table
        /// function, or a table's name.
        AsClause: Clause,
        /// `TO [db.]table` of a materialized view.
        ToClause: Clause,
        /// `POPULATE`, `EMPTY` or `CLONE`: whether a materialized view starts
        /// with the data its query selects, or a table created `AS …` with
        /// none or a copy.
        PopulateClause: Clause,
        /// `DEFINER = user` and `SQL SECURITY DEFINER|INVOKER|NONE`, either or
        /// both.
        SqlSecurityClause: Clause,
        /// `REFRESH EVERY|AFTER interval [OFFSET interval] [RANDOMIZE FOR
        /// interval]`, then any of a [`DependsOn`](Self::DependsOn), a
        /// [`SettingsClause`](Self::SettingsClause) and `APPEND
        /// [INCREMENTAL]`, of a materialized view.
        RefreshClause: Clause,
        /// `n UNIT [n UNIT …]` in a [`RefreshClause`](Self::RefreshClause):
        /// numbers and [`IntervalUnit`](Self::IntervalUnit)s.
        TimeInterval: Node,
        /// `DEPENDS ON` and table names separated by commas.
        DependsOn: List,
        /// A target table of a `TimeSeries` table: `SAMPLES`, `DATA`,
        /// `RECENT SAMPLES`, `TAGS`, `TIME RANGES`, `METRIC FAMILIES` or
        /// `METRICS`, then `INNER UUID '…'`, `INNER COLUMNS` and a
        /// [`TableElementList`](Self::TableElementList), `[INNER]` and the
        /// target's storage clauses, or the target's name.
        ViewTargetClause: Clause,
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
