use ctpipeline::value::CtValue;
use std::collections::HashMap;

/// Manages local variables for an executing workflow
#[derive(Debug, Default)]
pub struct WorkflowVars {
    inner: HashMap<String, CtValue>,
}

impl WorkflowVars {
    pub fn new() -> Self {
        Self {
            inner: HashMap::new(),
        }
    }

    /// Retrieve a variable by name
    pub fn get(&self, name: &str) -> Option<&CtValue> {
        self.inner.get(name)
    }

    /// Set a variable by name
    pub fn set(&mut self, name: String, val: CtValue) {
        self.inner.insert(name, val);
    }

    /// Expands variable references (e.g. `$var_name`) in the expression
    /// using their stringified values.
    pub fn expand_in_expr(&self, expr: &str) -> String {
        let mut out = String::with_capacity(expr.len());
        let chars: Vec<char> = expr.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            let ch = chars[i];
            if ch != '$' {
                out.push(ch);
                i += 1;
                continue;
            }

            if i + 1 < chars.len() && chars[i + 1] == '$' {
                out.push('$');
                i += 2;
                continue;
            }

            if i + 1 >= chars.len() || !is_var_start(chars[i + 1]) {
                out.push('$');
                i += 1;
                continue;
            }

            let mut j = i + 2;
            while j < chars.len() && is_var_continue(chars[j]) {
                j += 1;
            }

            let var_name: String = chars[i + 1..j].iter().collect();
            if let Some(v) = self.get(&var_name) {
                out.push_str(&ct_value_to_inline_text(v));
            } else {
                out.push('$');
                out.push_str(&var_name);
            }
            i = j;
        }
        out
    }
}

fn is_var_start(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphabetic()
}

fn is_var_continue(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphanumeric()
}

fn ct_value_to_inline_text(v: &CtValue) -> String {
    match v {
        CtValue::String(s) => s.clone(),
        CtValue::Int(i) => i.to_string(),
        CtValue::Float(f) => f.to_string(),
        CtValue::Bool(b) => b.to_string(),
        _ => format!("{v:?}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_workflow_vars_expand() {
        let mut vars = WorkflowVars::new();
        vars.set("target".into(), CtValue::String("prod".into()));
        vars.set("count".into(), CtValue::Int(5));

        let expr = "from json '{\"env\": \"$target\", \"limit\": $count}'";
        let expanded = vars.expand_in_expr(expr);
        assert_eq!(expanded, "from json '{\"env\": \"prod\", \"limit\": 5}'");
    }

    #[test]
    fn test_expand_prefers_longest_variable_name() {
        let mut vars = WorkflowVars::new();
        vars.set("a".into(), CtValue::String("X".into()));
        vars.set("ab".into(), CtValue::String("Y".into()));
        assert_eq!(vars.expand_in_expr("$ab + $a"), "Y + X");
    }

    #[test]
    fn test_expand_keeps_unknown_variable() {
        let vars = WorkflowVars::new();
        assert_eq!(vars.expand_in_expr("echo $unknown"), "echo $unknown");
    }

    #[test]
    fn test_expand_supports_dollar_escape() {
        let mut vars = WorkflowVars::new();
        vars.set("name".into(), CtValue::String("alice".into()));
        assert_eq!(vars.expand_in_expr("$$HOME $name"), "$HOME alice");
    }
}
