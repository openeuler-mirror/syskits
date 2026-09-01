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

    /// Remove a variable by name
    pub fn remove(&mut self, name: &str) -> Option<CtValue> {
        self.inner.remove(name)
    }

    /// 将表达式字符串中的 `$var` 引用替换为对应变量值的 DSL 安全字面量。
    ///
    /// 字符串值会被包裹为双引号字面量（同 ctdsl lexer 规则转义），以阻止
    /// 变量值中的空格、`|`、命令名等字符改变管线语义（命令注入）。
    /// 数值 / 布尔值不含特殊字符，直接内联。
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
                out.push_str(&ct_value_to_dsl_literal(v));
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

/// 将 CtValue 序列化为 DSL 安全的内联字面量。
///
/// 字符串值用双引号包裹并按 ctdsl lexer 规则转义，确保无论变量值内容
/// 如何都不会被 DSL 解析为命令名、管道符或其他语法结构。
/// 数值和布尔值不含特殊字符，直接转为字符串。
fn ct_value_to_dsl_literal(v: &CtValue) -> String {
    match v {
        CtValue::String(s) => dsl_quote_string(s),
        CtValue::Int(i) => i.to_string(),
        CtValue::Float(f) => f.to_string(),
        CtValue::Bool(b) => b.to_string(),
        _ => dsl_quote_string(&format!("{v:?}")),
    }
}

/// 对字符串内容生成 DSL 双引号字面量，转义与 ctdsl lexer lex_string 对应。
fn dsl_quote_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_workflow_vars_expand_string_quoted() {
        let mut vars = WorkflowVars::new();
        vars.set("target".into(), CtValue::String("prod".into()));
        vars.set("count".into(), CtValue::Int(5));

        // 字符串变量被展开为双引号字面量，数值直接内联
        let expr = "from json '{\"env\": $target, \"limit\": $count}'";
        let expanded = vars.expand_in_expr(expr);
        assert_eq!(expanded, "from json '{\"env\": \"prod\", \"limit\": 5}'");
    }

    #[test]
    fn test_expand_prefers_longest_variable_name() {
        let mut vars = WorkflowVars::new();
        vars.set("a".into(), CtValue::String("X".into()));
        vars.set("ab".into(), CtValue::String("Y".into()));
        assert_eq!(vars.expand_in_expr("$ab + $a"), r#""Y" + "X""#);
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
        assert_eq!(vars.expand_in_expr("$$HOME $name"), r#"$HOME "alice""#);
    }

    #[test]
    fn test_expand_string_with_pipe_cannot_inject_pipeline() {
        let mut vars = WorkflowVars::new();
        vars.set(
            "payload".into(),
            CtValue::String("foo | run-external rm -rf /".into()),
        );
        let expanded = vars.expand_in_expr("echo $payload");
        // 注入内容被包裹为字面量，| 不再是管道符
        assert_eq!(expanded, r#"echo "foo | run-external rm -rf /""#);
    }

    #[test]
    fn test_expand_string_escapes_backslash_and_quote() {
        let mut vars = WorkflowVars::new();
        vars.set("p".into(), CtValue::String(r#"a\"b"#.into()));
        assert_eq!(vars.expand_in_expr("$p"), r#""a\\\"b""#);
    }

    #[test]
    fn test_expand_int_and_bool_inline() {
        let mut vars = WorkflowVars::new();
        vars.set("n".into(), CtValue::Int(42));
        vars.set("flag".into(), CtValue::Bool(true));
        assert_eq!(vars.expand_in_expr("$n $flag"), "42 true");
    }
}
