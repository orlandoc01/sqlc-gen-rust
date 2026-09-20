//! Resolves column and relation references in a parsed query to the relations that declare
//! them, so a gated join's dependencies are checked by SQL scope rather than token spelling.
//!
//! Supported envelope: `SELECT` statements including nested subqueries, `EXISTS`, and set
//! operations; catalog tables; CTEs and derived tables as opaque relations whose output columns
//! are their projections' aliases or plain column names, renamed by a column alias list;
//! correlated references with local-shadows-outer resolution; `ON` constraints seeing only
//! relations declared before them; `LATERAL` derived tables seeing the relations before them.
//! An unqualified table name is looked up by name across every schema; several matches are an
//! error when the choice matters. A bare column that could belong to a relation whose columns
//! are unknown is an error when a gated relation could also provide it.
//!
//! Resolution errs toward finding a dependency: a non-lateral derived table also sees its
//! enclosing frames, an unaliased table matches a longer path ending in its name
//! (`public.orders.x` for `FROM orders`) as PostgreSQL allows, output aliases shadow input
//! columns only when the alias is a whole term, and a set operation's `ORDER BY` is not scanned
//! because it can only name output columns.

use std::ops::ControlFlow;

use sqlparser::{
    ast::{
        Cte, Expr, FunctionArg, FunctionArgExpr, FunctionArguments, GroupByExpr, Ident,
        JoinConstraint, JoinOperator, LimitClause, LockClause, ObjectName, ObjectNamePart, OrderBy,
        OrderByKind, Query, Select, SelectItem, SelectItemQualifiedWildcardKind, SetExpr, Spanned,
        Statement, TableAlias, TableFactor, TableWithJoins, Visit, Visitor,
    },
    tokenizer::{Location, Span},
};

use super::Dialect;
use crate::plugin;

/// The sqlc catalog's relations, borrowed from the request so building it copies no names.
#[derive(Default)]
pub(crate) struct Catalog<'a> {
    pub(crate) tables: Vec<CatalogTable<'a>>,
}

pub(crate) struct CatalogTable<'a> {
    pub(crate) schema: &'a str,
    pub(crate) name: &'a str,
    pub(crate) columns: Vec<&'a str>,
}

impl<'a> Catalog<'a> {
    pub(crate) fn from_plugin(catalog: &'a plugin::Catalog) -> Self {
        let tables = catalog
            .schemas
            .iter()
            .flat_map(|schema| schema.tables.iter().map(move |table| (schema, table)))
            .filter_map(|(schema, table)| {
                let rel = table.rel.as_ref()?;
                Some(CatalogTable {
                    schema: if rel.schema.is_empty() {
                        &schema.name
                    } else {
                        &rel.schema
                    },
                    name: &rel.name,
                    columns: table
                        .columns
                        .iter()
                        .map(|column| column.name.as_str())
                        .collect(),
                })
            })
            .collect();
        Self { tables }
    }
}

impl Dialect {
    /// PostgreSQL folds unquoted identifiers to lowercase and compares quoted ones exactly;
    /// SQLite and MySQL compare case-insensitively whether quoted or not.
    fn fold(self, ident: &Ident) -> String {
        match self {
            Self::PostgreSql if ident.quote_style.is_some() => ident.value.clone(),
            Self::PostgreSql | Self::MySql | Self::Sqlite => ident.value.to_lowercase(),
        }
    }

    /// A catalog name, which sqlc reports as stored, in the form `fold` produces.
    fn fold_catalog(self, name: &str) -> String {
        match self {
            Self::PostgreSql => name.to_string(),
            Self::MySql | Self::Sqlite => name.to_lowercase(),
        }
    }

    fn same(self, catalog_name: &str, folded: &str) -> bool {
        self.fold_catalog(catalog_name) == folded
    }
}

/// A join candidate that carries an annotation, identified by its relation's span.
pub(super) struct GatedJoin {
    pub(super) candidate: usize,
    pub(super) relation: Span,
}

pub(super) struct Reference {
    pub(super) candidate: usize,
    pub(super) at: Location,
    /// The name the reference uses for the relation, as declared.
    pub(super) alias: String,
    pub(super) what: What,
}

pub(super) enum What {
    /// `alias.column`, `alias.*`, a lock target, or a whole-row or table-valued use of the
    /// alias.
    Relation,
    /// A bare column name the gated relation provides.
    Column(String),
}

/// Every reference to a gated join's relation, in source order.
pub(super) fn references(
    statements: &[Statement],
    dialect: Dialect,
    catalog: &Catalog<'_>,
    gated: &[GatedJoin],
) -> Result<Vec<Reference>, String> {
    let mut resolver = Resolver {
        dialect,
        catalog,
        gated,
        frames: vec![Frame::default()],
        references: Vec::new(),
    };
    for statement in statements {
        resolver.scan(statement)?;
    }
    Ok(resolver.references)
}

/// Relation forms a gated join cannot use, named for the diagnostic.
pub(super) fn unsupported_factor(factor: &TableFactor) -> Option<&'static str> {
    match factor {
        TableFactor::Table { args: None, .. } | TableFactor::Derived { alias: Some(_), .. } => None,
        other => Some(describe_factor(other)),
    }
}

fn describe_factor(factor: &TableFactor) -> &'static str {
    match factor {
        TableFactor::Table { args: None, .. } => "a table",
        TableFactor::Table { .. }
        | TableFactor::TableFunction { .. }
        | TableFactor::Function { .. }
        | TableFactor::UNNEST { .. } => "a table function",
        TableFactor::Derived { alias: Some(_), .. } => "a derived table",
        TableFactor::Derived { .. } => "a derived table without an alias",
        TableFactor::NestedJoin { .. } => "a parenthesized join",
        _ => "an unsupported relation form",
    }
}

#[derive(Clone)]
enum Columns<'a> {
    Catalog(Vec<&'a CatalogTable<'a>>),
    Listed(Vec<String>),
    /// Leading columns renamed by an alias list over a relation whose remaining columns are
    /// unknown; carries the relation's description for diagnostics.
    Partial(Vec<String>, String),
    /// Carries a description of the relation for diagnostics.
    Unknown(String),
}

impl<'a> Columns<'a> {
    /// `None` when the relation's columns are unknown; `Err` names the schemas of an
    /// unqualified table whose definitions disagree.
    fn provides(&self, dialect: Dialect, column: &str) -> Result<Option<bool>, Vec<&str>> {
        match self {
            Self::Listed(columns) => Ok(Some(columns.iter().any(|name| name == column))),
            Self::Partial(known, _) => Ok(known.iter().any(|name| name == column).then_some(true)),
            Self::Unknown(_) => Ok(None),
            Self::Catalog(tables) => {
                let has = |table: &&CatalogTable<'_>| {
                    table.columns.iter().any(|name| dialect.same(name, column))
                };
                let first = has(&tables[0]);
                if tables.iter().all(|table| has(table) == first) {
                    Ok(Some(first))
                } else {
                    Err(tables.iter().map(|table| table.schema).collect())
                }
            }
        }
    }

    /// Why a column may belong to this relation without the plugin being able to tell.
    fn unknown(&self) -> Option<&str> {
        match self {
            Self::Partial(_, why) | Self::Unknown(why) => Some(why),
            Self::Catalog(_) | Self::Listed(_) => None,
        }
    }

    /// A column alias list (`o(oid, uid)`) renames the leading output columns and leaves the
    /// rest as declared, so a partial list keeps the trailing names.
    fn aliased(self, dialect: Dialect, alias: Option<&TableAlias>) -> Self {
        let names = alias
            .map(|alias| {
                alias
                    .columns
                    .iter()
                    .map(|column| dialect.fold(&column.name))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if names.is_empty() {
            return self;
        }
        let rename = |columns: Vec<String>| {
            names
                .iter()
                .cloned()
                .chain(columns.into_iter().skip(names.len()))
                .collect::<Vec<_>>()
        };
        match self {
            Self::Listed(columns) => Self::Listed(rename(columns)),
            Self::Partial(known, why) => Self::Partial(rename(known), why),
            Self::Unknown(why) => Self::Partial(names, why),
            Self::Catalog(tables) => {
                let mut lists = tables.iter().map(|table| {
                    rename(
                        table
                            .columns
                            .iter()
                            .map(|column| dialect.fold_catalog(column))
                            .collect(),
                    )
                });
                let first = lists.next().expect("a catalog relation lists a table");
                if lists.all(|list| list == first) {
                    Self::Listed(first)
                } else {
                    Self::Unknown(format!(
                        "table `{}`, which exists with different columns in schemas {}",
                        tables[0].name,
                        schemas(&tables)
                    ))
                }
            }
        }
    }
}

struct Relation<'a> {
    /// Folded alias; when present it is the only name that refers to the relation.
    alias: Option<String>,
    /// Folded object name parts of an unaliased table or CTE.
    name: Vec<String>,
    /// Source spelling for diagnostics.
    display: String,
    columns: Columns<'a>,
    gated: Option<usize>,
}

impl Relation<'_> {
    fn matches(&self, parts: &[String]) -> bool {
        match &self.alias {
            Some(alias) => parts.len() == 1 && parts[0] == *alias,
            None => {
                !self.name.is_empty() && (self.name.ends_with(parts) || parts.ends_with(&self.name))
            }
        }
    }

    fn gate(&self) -> Option<(usize, String)> {
        Some((self.gated?, self.display.clone()))
    }
}

#[derive(Default)]
struct Frame<'a> {
    relations: Vec<Relation<'a>>,
    ctes: Vec<Relation<'a>>,
}

/// The clauses after a query body that see the body's relations.
#[derive(Clone, Copy, Default)]
struct Tail<'q> {
    order_by: Option<&'q OrderBy>,
    limit: Option<&'q LimitClause>,
    fetch: Option<&'q sqlparser::ast::Fetch>,
    locks: &'q [LockClause],
}

struct Resolver<'a> {
    dialect: Dialect,
    catalog: &'a Catalog<'a>,
    gated: &'a [GatedJoin],
    frames: Vec<Frame<'a>>,
    references: Vec<Reference>,
}

impl<'a> Resolver<'a> {
    fn scan(&mut self, node: &impl Visit) -> Result<(), String> {
        self.scan_with(node, None)
    }

    fn scan_with(&mut self, node: &impl Visit, aliases: Option<&[String]>) -> Result<(), String> {
        let mut scan = Scan {
            resolver: self,
            aliases,
            nested: 0,
            opaque: 0,
        };
        match node.visit(&mut scan) {
            ControlFlow::Break(error) => Err(error),
            ControlFlow::Continue(()) => Ok(()),
        }
    }

    /// Every field is named so a sqlparser upgrade that adds one fails to compile here instead
    /// of silently skipping the references it may carry.
    fn walk_query(&mut self, query: &Query) -> Result<(), String> {
        let Query {
            with,
            body,
            order_by,
            limit_clause,
            fetch,
            locks,
            for_clause,
            settings,
            format_clause,
            pipe_operators,
        } = query;
        self.frames.push(Frame::default());
        if let Some(with) = with {
            for cte in &with.cte_tables {
                self.walk_query(&cte.query)?;
                let relation = self.cte(cte);
                self.frames
                    .last_mut()
                    .expect("walk_query pushed a frame")
                    .ctes
                    .push(relation);
            }
        }
        let tail = Tail {
            order_by: order_by.as_ref(),
            limit: limit_clause.as_ref(),
            fetch: fetch.as_ref(),
            locks,
        };
        self.walk_body(body, tail)?;
        self.scan(for_clause)?;
        self.scan(settings)?;
        self.scan(format_clause)?;
        self.scan(pipe_operators)?;
        self.frames.pop();
        Ok(())
    }

    fn walk_body(&mut self, body: &SetExpr, tail: Tail<'_>) -> Result<(), String> {
        match body {
            SetExpr::Select(select) => return self.walk_select(select, tail),
            SetExpr::Query(inner) => self.walk_query(inner)?,
            // The sides of a set operation are scopes of their own; its ORDER BY can only
            // name output columns, so it is not scanned.
            SetExpr::SetOperation { left, right, .. } => {
                self.walk_body(left, Tail::default())?;
                self.walk_body(right, Tail::default())?;
            }
            SetExpr::Values(values) => self.scan(values)?,
            SetExpr::Insert(statement)
            | SetExpr::Update(statement)
            | SetExpr::Delete(statement)
            | SetExpr::Merge(statement) => self.scan(statement)?,
            SetExpr::Table(_) => {}
        }
        self.scan_tail(tail)
    }

    fn walk_select(&mut self, select: &Select, tail: Tail<'_>) -> Result<(), String> {
        let Select {
            select_token: _,
            optimizer_hint,
            distinct,
            select_modifiers: _,
            top,
            top_before_distinct: _,
            projection,
            exclude,
            into,
            from,
            lateral_views,
            prewhere,
            selection,
            connect_by,
            group_by,
            cluster_by,
            distribute_by,
            sort_by,
            having,
            named_window,
            qualify,
            window_before_qualify: _,
            value_table_mode,
            flavor: _,
        } = select;
        self.frames.push(Frame::default());
        for from in from {
            self.walk_table_with_joins(from)?;
        }
        for item in projection {
            match item {
                SelectItem::QualifiedWildcard(
                    SelectItemQualifiedWildcardKind::ObjectName(name),
                    _,
                ) => self.relation_name(name),
                item => self.scan(item)?,
            }
        }
        self.scan(optimizer_hint)?;
        self.scan(distinct)?;
        self.scan(top)?;
        self.scan(exclude)?;
        self.scan(into)?;
        self.scan(lateral_views)?;
        self.scan(prewhere)?;
        self.scan(selection)?;
        self.scan(connect_by)?;
        let aliases = projection
            .iter()
            .filter_map(|item| match item {
                SelectItem::ExprWithAlias { alias, .. } => Some(self.dialect.fold(alias)),
                _ => None,
            })
            .collect::<Vec<_>>();
        self.group_by(group_by, &aliases)?;
        self.scan(cluster_by)?;
        self.scan(distribute_by)?;
        self.scan(sort_by)?;
        self.having(having.as_ref(), &aliases)?;
        self.scan(named_window)?;
        self.scan(qualify)?;
        self.scan(value_table_mode)?;
        if let Some(order_by) = tail.order_by {
            self.order_by(order_by, &aliases)?;
        }
        self.scan_tail(tail)?;
        self.frames.pop();
        Ok(())
    }

    fn scan_tail(&mut self, tail: Tail<'_>) -> Result<(), String> {
        if let Some(limit) = tail.limit {
            self.scan(limit)?;
        }
        if let Some(fetch) = tail.fetch {
            self.scan(fetch)?;
        }
        for lock in tail.locks {
            if let Some(name) = &lock.of {
                self.relation_name(name);
            }
        }
        Ok(())
    }

    /// Output aliases take precedence in `ORDER BY` only when the alias is the whole term; an
    /// identifier inside an expression is an input column on every supported engine.
    fn order_by(&mut self, order_by: &OrderBy, aliases: &[String]) -> Result<(), String> {
        match &order_by.kind {
            OrderByKind::All(_) => {}
            OrderByKind::Expressions(terms) => {
                for term in terms {
                    match &term.expr {
                        Expr::Identifier(ident) if aliases.contains(&self.dialect.fold(ident)) => {
                            self.scan(&term.with_fill)?;
                        }
                        _ => self.scan(term)?,
                    }
                }
            }
        }
        self.scan(&order_by.interpolate)
    }

    /// A whole-term `GROUP BY` name is an input column first and an output alias second.
    fn group_by(&mut self, group_by: &GroupByExpr, aliases: &[String]) -> Result<(), String> {
        match group_by {
            GroupByExpr::All(modifiers) => self.scan(modifiers),
            GroupByExpr::Expressions(terms, modifiers) => {
                for term in terms {
                    match term {
                        Expr::Identifier(ident) => self.bare(ident, Some(aliases))?,
                        term => self.scan(term)?,
                    }
                }
                self.scan(modifiers)
            }
        }
    }

    /// PostgreSQL never resolves output aliases in `HAVING`; SQLite and MySQL do, after input
    /// columns, and only whole terms are trusted to be aliases.
    fn having(&mut self, having: Option<&Expr>, aliases: &[String]) -> Result<(), String> {
        let Some(having) = having else {
            return Ok(());
        };
        match self.dialect {
            Dialect::PostgreSql => self.scan(having),
            Dialect::MySql | Dialect::Sqlite => self.scan_with(having, Some(aliases)),
        }
    }

    fn walk_table_with_joins(&mut self, from: &TableWithJoins) -> Result<(), String> {
        self.walk_factor(&from.relation, None)?;
        for join in &from.joins {
            let gated = self
                .gated
                .iter()
                .find(|gated| gated.relation == join.relation.span())
                .map(|gated| gated.candidate);
            // `USING (c)` binds `c` on the left side, which may be a gated relation.
            if let Some(JoinConstraint::Using(names)) = constraint(&join.join_operator) {
                for name in names {
                    if let Some(ObjectNamePart::Identifier(ident)) = name.0.last() {
                        self.bare(ident, None)?;
                    }
                }
            }
            self.walk_factor(&join.relation, gated)?;
            self.scan(&join.join_operator)?;
        }
        Ok(())
    }

    fn walk_factor(&mut self, factor: &TableFactor, gated: Option<usize>) -> Result<(), String> {
        let relation = match factor {
            TableFactor::Table {
                name, alias, args, ..
            } => {
                self.scan(args)?;
                self.table(name, alias.as_ref(), args.is_some(), gated)?
            }
            TableFactor::Derived {
                lateral,
                subquery,
                alias,
                ..
            } => {
                // A derived table sees the enclosing queries but not its siblings, unless
                // it is LATERAL.
                if *lateral {
                    self.walk_query(subquery)?;
                } else {
                    let frame = self.frames.pop();
                    self.walk_query(subquery)?;
                    self.frames.extend(frame);
                }
                Relation {
                    alias: alias.as_ref().map(|alias| self.dialect.fold(&alias.name)),
                    name: Vec::new(),
                    display: alias.as_ref().map_or_else(
                        || "a derived table".to_string(),
                        |alias| alias.name.value.clone(),
                    ),
                    columns: self
                        .body_columns(&subquery.body)
                        .aliased(self.dialect, alias.as_ref()),
                    gated,
                }
            }
            TableFactor::NestedJoin {
                table_with_joins,
                alias: None,
            } => return self.walk_table_with_joins(table_with_joins),
            other => {
                self.scan(other)?;
                let alias = alias_of(other);
                Relation {
                    alias: alias.map(|alias| self.dialect.fold(&alias.name)),
                    name: Vec::new(),
                    display: alias.map_or_else(
                        || "a relation".to_string(),
                        |alias| alias.name.value.clone(),
                    ),
                    columns: Columns::Unknown(describe_factor(other).to_string())
                        .aliased(self.dialect, alias),
                    gated,
                }
            }
        };
        self.frames
            .last_mut()
            .expect("walk_select pushed a frame")
            .relations
            .push(relation);
        Ok(())
    }

    fn table(
        &self,
        name: &ObjectName,
        alias: Option<&TableAlias>,
        is_function: bool,
        gated: Option<usize>,
    ) -> Result<Relation<'a>, String> {
        let idents = name
            .0
            .iter()
            .map(|part| match part {
                ObjectNamePart::Identifier(ident) => Some(ident),
                ObjectNamePart::Function(_) => None,
            })
            .collect::<Option<Vec<_>>>()
            .unwrap_or_default();
        let parts = idents
            .iter()
            .map(|ident| self.dialect.fold(ident))
            .collect::<Vec<_>>();
        let table = idents
            .last()
            .map_or_else(|| name.to_string(), |ident| ident.value.clone());
        let display = alias.map_or_else(|| table.clone(), |alias| alias.name.value.clone());
        let columns = if is_function || parts.is_empty() {
            Columns::Unknown("a table function".to_string())
        } else if let Some(cte) = self.cte_columns(&parts) {
            cte
        } else {
            let tables = self.catalog_tables(&parts);
            if gated.is_some() && tables.len() > 1 {
                return Err(format!(
                    "gated JOIN table `{table}` on line {} exists in schemas {}; qualify it with its schema",
                    idents[0].span.start.line,
                    schemas(&tables)
                ));
            }
            if tables.is_empty() {
                Columns::Unknown(format!(
                    "table `{table}`, which the sqlc catalog does not describe"
                ))
            } else {
                Columns::Catalog(tables)
            }
        };
        Ok(Relation {
            alias: alias.map(|alias| self.dialect.fold(&alias.name)),
            name: parts,
            display,
            columns: columns.aliased(self.dialect, alias),
            gated,
        })
    }

    fn cte_columns(&self, parts: &[String]) -> Option<Columns<'a>> {
        let [name] = parts else {
            return None;
        };
        self.frames
            .iter()
            .rev()
            .flat_map(|frame| frame.ctes.iter().rev())
            .find(|cte| cte.name[0] == *name)
            .map(|cte| cte.columns.clone())
    }

    fn catalog_tables(&self, parts: &[String]) -> Vec<&'a CatalogTable<'a>> {
        let (schema, name) = match parts {
            [.., schema, name] => (Some(schema), name),
            [name] => (None, name),
            [] => return Vec::new(),
        };
        self.catalog
            .tables
            .iter()
            .filter(|table| self.dialect.same(table.name, name))
            .filter(|table| {
                schema.is_none_or(|schema| {
                    table.schema.is_empty() || self.dialect.same(table.schema, schema)
                })
            })
            .collect()
    }

    fn cte(&self, cte: &Cte) -> Relation<'a> {
        Relation {
            alias: None,
            name: vec![self.dialect.fold(&cte.alias.name)],
            display: cte.alias.name.value.clone(),
            columns: self
                .body_columns(&cte.query.body)
                .aliased(self.dialect, Some(&cte.alias)),
            gated: None,
        }
    }

    /// Output column names of a subquery: projection aliases and plain column names. An
    /// unnamed expression gets an engine-specific name and is left out.
    fn body_columns(&self, body: &SetExpr) -> Columns<'a> {
        match body {
            SetExpr::Select(select) => {
                let mut columns = Vec::new();
                for item in &select.projection {
                    match item {
                        SelectItem::ExprWithAlias { alias, .. } => {
                            columns.push(self.dialect.fold(alias));
                        }
                        SelectItem::UnnamedExpr(Expr::Identifier(ident)) => {
                            columns.push(self.dialect.fold(ident));
                        }
                        SelectItem::UnnamedExpr(Expr::CompoundIdentifier(parts)) => {
                            if let Some(last) = parts.last() {
                                columns.push(self.dialect.fold(last));
                            }
                        }
                        SelectItem::UnnamedExpr(_) => {}
                        SelectItem::Wildcard(_) | SelectItem::QualifiedWildcard(..) => {
                            return Columns::Unknown("a subquery selecting `*`".to_string());
                        }
                    }
                }
                Columns::Listed(columns)
            }
            SetExpr::Query(inner) => self.body_columns(&inner.body),
            SetExpr::SetOperation { left, .. } => self.body_columns(left),
            _ => Columns::Unknown("a subquery without a projection".to_string()),
        }
    }

    fn relation(&self, parts: &[String]) -> Option<&Relation<'a>> {
        self.frames
            .iter()
            .rev()
            .flat_map(|frame| frame.relations.iter().rev())
            .find(|relation| relation.matches(parts))
    }

    fn record(&mut self, gated: Option<(usize, String)>, at: Location, what: What) {
        if let Some((candidate, alias)) = gated {
            self.references.push(Reference {
                candidate,
                at,
                alias,
                what,
            });
        }
    }

    /// An object name that refers to a whole relation: `alias.*` or a lock target.
    fn relation_name(&mut self, name: &ObjectName) {
        let parts = self.parts(name);
        if let Some(part) = name.0.first() {
            let gated = self.relation(&parts).and_then(Relation::gate);
            self.record(gated, part.span().start, What::Relation);
        }
    }

    fn parts(&self, name: &ObjectName) -> Vec<String> {
        name.0
            .iter()
            .filter_map(|part| match part {
                ObjectNamePart::Identifier(ident) => Some(self.dialect.fold(ident)),
                ObjectNamePart::Function(_) => None,
            })
            .collect()
    }

    /// `a.b`, `schema.table.column`, or a field access on a composite column: the longest
    /// leading path naming a visible relation wins.
    fn compound(&mut self, idents: &[Ident]) {
        let parts = idents
            .iter()
            .map(|ident| self.dialect.fold(ident))
            .collect::<Vec<_>>();
        let found = (1..parts.len())
            .rev()
            .find_map(|length| self.relation(&parts[..length]))
            .and_then(Relation::gate);
        self.record(found, idents[0].span.start, What::Relation);
    }

    /// A bare identifier binds in the innermost scope that can: to a column a relation there
    /// provides, else to a relation of that scope as a whole-row or table-valued reference.
    /// A relation whose columns are unknown stops the search when a gated relation could
    /// provide the name. An output alias applies only where the engine allows it and no input
    /// column binds first.
    fn bare(&mut self, ident: &Ident, aliases: Option<&[String]>) -> Result<(), String> {
        let name = self.dialect.fold(ident);
        let at = ident.span.start;
        let alias = aliases.is_some_and(|aliases| aliases.contains(&name));
        for depth in (0..self.frames.len()).rev() {
            match self.bind_in_frame(depth, &name) {
                Ok(Binding::Column(gated)) => {
                    self.record(gated, at, What::Column(ident.value.clone()));
                    return Ok(());
                }
                Ok(Binding::Relation(gated)) => {
                    self.record(gated, at, What::Relation);
                    return Ok(());
                }
                Ok(Binding::Outer) => {}
                Ok(Binding::Unknown(_)) | Err(_) if alias => return Ok(()),
                Ok(Binding::Unknown(why)) => {
                    return Err(format!(
                        "column `{}` on line {} cannot be resolved because it may belong to {why}; qualify it with its table alias",
                        ident.value, at.line
                    ));
                }
                Err((display, schemas)) => {
                    return Err(format!(
                        "column `{}` on line {} may belong to table `{display}`, which exists with different columns in schemas {}; qualify the table with its schema",
                        ident.value,
                        at.line,
                        schemas
                            .iter()
                            .map(|schema| format!("`{schema}`"))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                }
            }
        }
        Ok(())
    }

    /// `Err` carries the display name and schemas of an unqualified table whose definitions
    /// disagree about the column.
    fn bind_in_frame(&self, depth: usize, folded: &str) -> Result<Binding, (String, Vec<&str>)> {
        let frame = &self.frames[depth];
        let mut gated = None;
        let mut provided = false;
        let mut unknown = None;
        for relation in &frame.relations {
            match relation.columns.provides(self.dialect, folded) {
                Ok(Some(true)) => match relation.gate() {
                    Some(gate) => gated = Some(gate),
                    None => provided = true,
                },
                Ok(Some(false)) => {}
                Ok(None) => unknown = unknown.or_else(|| relation.columns.unknown()),
                Err(schemas) => return Err((relation.display.clone(), schemas)),
            }
        }
        if gated.is_some() {
            return Ok(Binding::Column(gated));
        }
        if provided {
            return Ok(Binding::Column(None));
        }
        if let Some(relation) = frame
            .relations
            .iter()
            .rev()
            .find(|relation| relation.matches(std::slice::from_ref(&folded.to_string())))
        {
            return Ok(Binding::Relation(relation.gate()));
        }
        Ok(match unknown {
            Some(why) if self.gated_may_provide(depth, folded) => Binding::Unknown(why.to_string()),
            _ => Binding::Outer,
        })
    }

    /// Whether a gated relation visible from `depth` outward could provide `column`.
    fn gated_may_provide(&self, depth: usize, column: &str) -> bool {
        self.frames[..=depth]
            .iter()
            .flat_map(|frame| &frame.relations)
            .filter(|relation| relation.gated.is_some())
            .any(|relation| {
                !matches!(
                    relation.columns.provides(self.dialect, column),
                    Ok(Some(false))
                )
            })
    }
}

fn constraint(operator: &JoinOperator) -> Option<&JoinConstraint> {
    match operator {
        JoinOperator::Join(constraint)
        | JoinOperator::Inner(constraint)
        | JoinOperator::Left(constraint)
        | JoinOperator::LeftOuter(constraint)
        | JoinOperator::Right(constraint)
        | JoinOperator::RightOuter(constraint)
        | JoinOperator::FullOuter(constraint)
        | JoinOperator::CrossJoin(constraint)
        | JoinOperator::Semi(constraint)
        | JoinOperator::LeftSemi(constraint)
        | JoinOperator::RightSemi(constraint)
        | JoinOperator::Anti(constraint)
        | JoinOperator::LeftAnti(constraint)
        | JoinOperator::RightAnti(constraint)
        | JoinOperator::AsOf { constraint, .. } => Some(constraint),
        _ => None,
    }
}

enum Binding {
    /// A column of a relation in this scope; `Some` names the gated relation.
    Column(Option<(usize, String)>),
    /// A whole-row or table-valued use of a relation in this scope.
    Relation(Option<(usize, String)>),
    /// A relation whose columns are unknown may provide it, and so may a gated relation.
    Unknown(String),
    /// Not bound in this scope; the enclosing scope is next.
    Outer,
}

fn schemas(tables: &[&CatalogTable<'_>]) -> String {
    tables
        .iter()
        .map(|table| format!("`{}`", table.schema))
        .collect::<Vec<_>>()
        .join(", ")
}

fn alias_of(factor: &TableFactor) -> Option<&TableAlias> {
    match factor {
        TableFactor::Table { alias, .. }
        | TableFactor::Derived { alias, .. }
        | TableFactor::TableFunction { alias, .. }
        | TableFactor::Function { alias, .. }
        | TableFactor::UNNEST { alias, .. }
        | TableFactor::JsonTable { alias, .. }
        | TableFactor::OpenJsonTable { alias, .. }
        | TableFactor::NestedJoin { alias, .. }
        | TableFactor::Pivot { alias, .. }
        | TableFactor::Unpivot { alias, .. }
        | TableFactor::MatchRecognize { alias, .. } => alias.as_ref(),
        _ => None,
    }
}

struct Scan<'r, 'a> {
    resolver: &'r mut Resolver<'a>,
    aliases: Option<&'r [String]>,
    /// Nested queries are walked with their own scopes; the visitor's own descent into them
    /// is ignored.
    nested: usize,
    /// Depth inside expressions that are not plain boolean or comparison combinators: an
    /// identifier there is never a whole term, so output aliases do not apply to it.
    opaque: usize,
}

/// Expressions an output alias may be a whole term of: the alias itself, and the boolean and
/// comparison operators that combine terms.
fn transparent(expr: &Expr) -> bool {
    matches!(
        expr,
        Expr::Identifier(_)
            | Expr::CompoundIdentifier(_)
            | Expr::Value(_)
            | Expr::Nested(_)
            | Expr::BinaryOp { .. }
            | Expr::UnaryOp { .. }
            | Expr::IsNull(_)
            | Expr::IsNotNull(_)
            | Expr::IsTrue(_)
            | Expr::IsNotTrue(_)
            | Expr::IsFalse(_)
            | Expr::IsNotFalse(_)
            | Expr::IsUnknown(_)
            | Expr::IsNotUnknown(_)
            | Expr::IsDistinctFrom(..)
            | Expr::IsNotDistinctFrom(..)
            | Expr::Between { .. }
            | Expr::InList { .. }
            | Expr::Like { .. }
            | Expr::ILike { .. }
    )
}

impl Visitor for Scan<'_, '_> {
    type Break = String;

    fn pre_visit_query(&mut self, query: &Query) -> ControlFlow<String> {
        if self.nested == 0
            && let Err(error) = self.resolver.walk_query(query)
        {
            return ControlFlow::Break(error);
        }
        self.nested += 1;
        ControlFlow::Continue(())
    }

    fn post_visit_query(&mut self, _query: &Query) -> ControlFlow<String> {
        self.nested -= 1;
        ControlFlow::Continue(())
    }

    fn pre_visit_expr(&mut self, expr: &Expr) -> ControlFlow<String> {
        if self.nested > 0 {
            return ControlFlow::Continue(());
        }
        let aliases = if self.opaque == 0 { self.aliases } else { None };
        if !transparent(expr) {
            self.opaque += 1;
        }
        match expr {
            Expr::Identifier(ident) => {
                if let Err(error) = self.resolver.bare(ident, aliases) {
                    return ControlFlow::Break(error);
                }
            }
            Expr::CompoundIdentifier(idents) => self.resolver.compound(idents),
            Expr::Function(function) => {
                if let FunctionArguments::List(list) = &function.args {
                    for arg in &list.args {
                        if let FunctionArg::Unnamed(FunctionArgExpr::QualifiedWildcard(name))
                        | FunctionArg::Named {
                            arg: FunctionArgExpr::QualifiedWildcard(name),
                            ..
                        }
                        | FunctionArg::ExprNamed {
                            arg: FunctionArgExpr::QualifiedWildcard(name),
                            ..
                        } = arg
                        {
                            self.resolver.relation_name(name);
                        }
                    }
                }
            }
            _ => {}
        }
        ControlFlow::Continue(())
    }

    fn post_visit_expr(&mut self, expr: &Expr) -> ControlFlow<String> {
        if self.nested == 0 && !transparent(expr) {
            self.opaque -= 1;
        }
        ControlFlow::Continue(())
    }
}

#[cfg(test)]
mod tests;
