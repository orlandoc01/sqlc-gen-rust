//! Control slot allocation: every `:flag`, `:switch` field, and switch choice claims one
//! argument slot after the SQL parameters, and `references` reports the plan's control layout.

use std::collections::{BTreeSet, HashMap, HashSet};

use super::{Directive, FlagParam, References};

#[derive(Clone, Copy, PartialEq, Eq)]
enum SlotKind {
    Flag,
    SwitchField,
    Choice,
}

impl SlotKind {
    fn describe(self) -> &'static str {
        match self {
            Self::Flag => "a `-- :flag` name",
            Self::SwitchField => "a `-- :switch` field",
            Self::Choice => "a `-- :switch` choice",
        }
    }
}

struct SlotTable<'a> {
    /// Number of SQL argument slots; controls are allocated after them, so this must not be
    /// derived from the name map (sqlc can report several parameters under one name).
    arg_count: usize,
    param_by_name: HashMap<&'a str, usize>,
    ambiguous: HashSet<&'a str>,
    kinds: HashMap<String, SlotKind>,
    references: References,
}

impl SlotTable<'_> {
    fn claim(&mut self, name: &str, kind: SlotKind, switch: Option<usize>) -> Result<(), String> {
        if self.param_by_name.contains_key(name) {
            return Err(format!(
                "`{name}` is a SQL parameter and cannot be {}; use `-- :if @{name}` to make it optional",
                kind.describe()
            ));
        }
        match self.kinds.get(name).copied() {
            Some(SlotKind::Flag) if kind == SlotKind::Flag => Ok(()),
            Some(existing) => Err(format!(
                "`{name}` is already {}; it cannot also be {}",
                existing.describe(),
                kind.describe()
            )),
            None => {
                let index = self.arg_count + self.references.flag_params.len();
                self.kinds.insert(name.to_string(), kind);
                if kind != SlotKind::SwitchField {
                    self.references
                        .arg_index_by_name
                        .insert(name.to_string(), index);
                    self.references.flag_params.push(FlagParam {
                        name: name.to_string(),
                        switch,
                    });
                }
                Ok(())
            }
        }
    }

    fn condition(&mut self, name: &str) -> Result<(), String> {
        if self.ambiguous.contains(name) {
            return Err(format!(
                "`-- :if @{name}` is ambiguous: several SQL parameters are named `{name}`"
            ));
        }
        let number = *self.param_by_name.get(name).ok_or_else(|| {
            format!(
                "`-- :if @{name}` names no SQL parameter; use `-- :flag @{name}` for a boolean toggle"
            )
        })?;
        self.references
            .arg_index_by_name
            .insert(name.to_string(), number - 1);
        self.references.conditional_param_numbers.push(number);
        Ok(())
    }
}

pub(crate) fn references<'a>(
    params: &[(String, usize)],
    directives: impl Iterator<Item = &'a Directive>,
) -> Result<References, String> {
    let mut param_by_name = HashMap::new();
    let ambiguous = params
        .iter()
        .filter(|(name, number)| param_by_name.insert(name.as_str(), *number).is_some())
        .map(|(name, _)| name.as_str())
        .collect();
    let mut table = SlotTable {
        arg_count: params.len(),
        param_by_name,
        ambiguous,
        kinds: HashMap::new(),
        references: References {
            conditional_param_numbers: Vec::new(),
            flag_params: Vec::new(),
            switches: Vec::new(),
            arg_index_by_name: HashMap::new(),
        },
    };
    for directive in directives {
        match directive {
            Directive::If(names) => names.iter().try_for_each(|name| table.condition(name))?,
            Directive::Flag(names) => names
                .iter()
                .try_for_each(|name| table.claim(name, SlotKind::Flag, None))?,
            Directive::Case(_) => {}
            Directive::Switch(switch) => {
                let index = table.references.switches.len();
                table.claim(&switch.field, SlotKind::SwitchField, None)?;
                switch
                    .choices
                    .iter()
                    .try_for_each(|choice| table.claim(choice, SlotKind::Choice, Some(index)))?;
                table.references.switches.push(switch.clone());
            }
        }
    }
    let numbers = std::mem::take(&mut table.references.conditional_param_numbers)
        .into_iter()
        .collect::<BTreeSet<_>>();
    table.references.conditional_param_numbers = numbers.into_iter().collect();
    Ok(table.references)
}
