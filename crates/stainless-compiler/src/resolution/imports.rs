use std::collections::BTreeMap;

use crate::Diagnostic;
use crate::ast::{Item, SourceFile, Span};

#[derive(Clone, Debug, Default)]
pub(super) struct ImportTable {
    scopes: BTreeMap<Vec<String>, ImportScope>,
}

#[derive(Clone, Debug, Default)]
struct ImportScope {
    aliases: BTreeMap<String, Vec<ImportedPath>>,
    globs: Vec<ImportedPath>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ImportedPath {
    path: Vec<String>,
    span: Span,
}

#[derive(Clone, Debug)]
struct ExpandedImport {
    path: Vec<String>,
    alias: Option<String>,
    glob: bool,
}

impl ImportTable {
    pub(super) fn build(source: &SourceFile, diagnostics: &mut Vec<Diagnostic>) -> Self {
        let mut table = Self::default();
        table.collect_items(&source.items, &mut Vec::new(), diagnostics);
        table
    }

    pub(super) fn candidates(&self, namespace: &[String], name: &str) -> Vec<Vec<String>> {
        for depth in (0..=namespace.len()).rev() {
            let scope_path = namespace[..depth].to_vec();
            let Some(scope) = self.scopes.get(&scope_path) else {
                continue;
            };
            let mut candidates = scope
                .aliases
                .get(name)
                .into_iter()
                .flatten()
                .map(|import| import.path.clone())
                .collect::<Vec<_>>();
            candidates.extend(scope.globs.iter().map(|import| {
                let mut path = import.path.clone();
                path.push(name.to_owned());
                path
            }));
            candidates.sort();
            candidates.dedup();
            if !candidates.is_empty() {
                return candidates;
            }
        }
        Vec::new()
    }

    fn collect_items(
        &mut self,
        items: &[Item],
        namespace: &mut Vec<String>,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        for item in items {
            match item {
                Item::Use(declaration) => {
                    let imports = match expand_import(&declaration.path, namespace) {
                        Ok(imports) => imports,
                        Err(message) => {
                            diagnostics.push(Diagnostic::semantic(
                                "RES001",
                                message,
                                declaration.span,
                            ));
                            continue;
                        }
                    };
                    for import in imports {
                        self.insert(namespace, import, declaration.span, diagnostics);
                    }
                }
                Item::Namespace(child) => {
                    namespace.push(child.name.clone());
                    self.collect_items(&child.items, namespace, diagnostics);
                    namespace.pop();
                }
                Item::Struct(_) | Item::Function(_) => {}
            }
        }
    }

    fn insert(
        &mut self,
        namespace: &[String],
        import: ExpandedImport,
        span: Span,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        let scope = self.scopes.entry(namespace.to_vec()).or_default();
        let imported = ImportedPath {
            path: import.path,
            span,
        };
        if import.glob {
            if !scope
                .globs
                .iter()
                .any(|existing| existing.path == imported.path)
            {
                scope.globs.push(imported);
            }
            return;
        }

        let alias = import
            .alias
            .or_else(|| imported.path.last().cloned())
            .unwrap_or_else(|| "<missing>".to_owned());
        let entries = scope.aliases.entry(alias.clone()).or_default();
        if entries
            .iter()
            .any(|existing| existing.path != imported.path)
        {
            diagnostics.push(Diagnostic::semantic(
                "RES002",
                format!("imported name `{alias}` is ambiguous in this namespace"),
                span,
            ));
        }
        if !entries
            .iter()
            .any(|existing| existing.path == imported.path)
        {
            entries.push(imported);
        }
    }
}

fn expand_import(text: &str, namespace: &[String]) -> Result<Vec<ExpandedImport>, String> {
    let mut raw = Vec::new();
    expand_tree(text.trim(), "", &mut raw)?;
    raw.into_iter()
        .map(|mut import| {
            import.path = anchor_path(&import.path, namespace)?;
            Ok(import)
        })
        .collect()
}

fn expand_tree(tree: &str, prefix: &str, output: &mut Vec<ExpandedImport>) -> Result<(), String> {
    let tree = tree.trim();
    if let Some(open) = tree.find('{') {
        let close =
            matching_close_brace(tree, open).ok_or_else(|| "unclosed import group".to_owned())?;
        if !tree[close + 1..].trim().is_empty() {
            return Err("unexpected text after import group".to_owned());
        }
        let base = join_path(prefix, tree[..open].trim().trim_end_matches("::"));
        for member in split_group(&tree[open + 1..close])? {
            expand_tree(member, &base, output)?;
        }
        return Ok(());
    }

    let complete = join_path(prefix, tree);
    let (path, alias) = complete
        .rsplit_once(" as ")
        .map_or((complete.as_str(), None), |(path, alias)| {
            (path.trim(), Some(alias.trim().to_owned()))
        });
    let glob = path.ends_with("::*");
    let path = path.trim_end_matches("::*");
    if path.is_empty() || alias.as_ref().is_some_and(String::is_empty) {
        return Err("expected a path in import declaration".to_owned());
    }
    output.push(ExpandedImport {
        path: path.split("::").map(str::to_owned).collect(),
        alias,
        glob,
    });
    Ok(())
}

fn matching_close_brace(text: &str, open: usize) -> Option<usize> {
    let mut depth = 0_u32;
    for (offset, byte) in text.as_bytes()[open..].iter().enumerate() {
        match byte {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(open + offset);
                }
            }
            _ => {}
        }
    }
    None
}

fn split_group(contents: &str) -> Result<Vec<&str>, String> {
    let mut members = Vec::new();
    let mut depth = 0_u32;
    let mut start = 0;
    for (index, byte) in contents.bytes().enumerate() {
        match byte {
            b'{' => depth += 1,
            b'}' => {
                depth = depth
                    .checked_sub(1)
                    .ok_or_else(|| "unexpected `}` in import group".to_owned())?;
            }
            b',' if depth == 0 => {
                let member = contents[start..index].trim();
                if !member.is_empty() {
                    members.push(member);
                }
                start = index + 1;
            }
            _ => {}
        }
    }
    let member = contents[start..].trim();
    if !member.is_empty() {
        members.push(member);
    }
    Ok(members)
}

fn join_path(prefix: &str, suffix: &str) -> String {
    match (prefix.is_empty(), suffix.is_empty()) {
        (_, true) => prefix.to_owned(),
        (true, false) => suffix.to_owned(),
        (false, false) => format!(
            "{}::{}",
            prefix.trim_end_matches("::"),
            suffix.trim_start_matches("::")
        ),
    }
}

fn anchor_path(path: &[String], namespace: &[String]) -> Result<Vec<String>, String> {
    let Some(first) = path.first().map(String::as_str) else {
        return Err("expected a path in import declaration".to_owned());
    };
    match first {
        "crate" => Ok(path[1..].to_vec()),
        "self" => {
            let mut result = namespace.to_vec();
            result.extend_from_slice(&path[1..]);
            Ok(result)
        }
        "super" => {
            let Some((_, parent)) = namespace.split_last() else {
                return Err("`super` cannot be used at crate root".to_owned());
            };
            let mut result = parent.to_vec();
            result.extend_from_slice(&path[1..]);
            Ok(result)
        }
        _ => Ok(path.to_vec()),
    }
}
