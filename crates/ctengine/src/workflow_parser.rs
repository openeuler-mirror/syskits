use crate::workflow::{WorkflowScript, WorkflowStage};

#[derive(Debug, thiserror::Error)]
pub enum WorkflowParseError {
    #[error("Failed to parse YAML workflow: {0}")]
    YamlError(#[from] serde_yaml::Error),
    #[error("Failed to parse Text workflow: {0}")]
    TextError(String),
}

#[derive(serde::Deserialize)]
struct WorkflowYamlStage {
    name: String,
    expr: Option<String>,
    #[serde(rename = "if")]
    if_cond: Option<String>,
    else_expr: Option<String>,
    foreach: Option<String>,
    var: Option<String>,
    timeout_ms: Option<u64>,
    retry: Option<u32>,
    on_failure: Option<serde_yaml::Value>,
    #[serde(default)]
    checkpoint: bool,
}

#[derive(serde::Deserialize)]
struct WorkflowYamlRoot {
    stages: Vec<WorkflowYamlStage>,
}

pub fn parse_yaml_workflow(src: &str) -> Result<WorkflowScript, WorkflowParseError> {
    let root: WorkflowYamlRoot = serde_yaml::from_str(src)?;
    let stages = root
        .stages
        .into_iter()
        .map(|s| -> Result<WorkflowStage, WorkflowParseError> {
            Ok(WorkflowStage {
                name: s.name,
                expr: s.expr,
                if_cond: s.if_cond,
                else_expr: s.else_expr,
                foreach: s.foreach,
                var: s.var,
                timeout_ms: s.timeout_ms,
                retry: s.retry,
                on_failure: match s.on_failure {
                    Some(v) => parse_on_failure(v)?,
                    None => Default::default(),
                },
                checkpoint: s.checkpoint,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;

    Ok(WorkflowScript { stages })
}

fn parse_on_failure(
    v: serde_yaml::Value,
) -> Result<crate::workflow::OnFailure, WorkflowParseError> {
    match v {
        serde_yaml::Value::String(s) => {
            let normalized = s.trim().to_lowercase();
            if normalized == "fail" {
                return Ok(crate::workflow::OnFailure::Fail);
            }
            if normalized == "continue" {
                return Ok(crate::workflow::OnFailure::Continue);
            }
            if normalized.starts_with("goto(") && normalized.ends_with(')') {
                let trimmed = s.trim();
                let target = trimmed[5..trimmed.len() - 1].trim();

                if target.is_empty() {
                    return Err(WorkflowParseError::TextError(
                        "on_failure 'goto()' requires a non-empty stage name".into(),
                    ));
                }
                return Ok(crate::workflow::OnFailure::Goto(target.to_string()));
            }
            Err(WorkflowParseError::TextError(format!(
                "invalid on_failure value '{s}'; expected fail|continue|goto(stage)"
            )))
        }
        serde_yaml::Value::Mapping(m) => {
            let key = serde_yaml::Value::String("goto".to_string());
            if let Some(value) = m.get(&key) {
                if let Some(target) = value.as_str() {
                    if target.trim().is_empty() {
                        return Err(WorkflowParseError::TextError(
                            "on_failure.goto requires a non-empty stage name".into(),
                        ));
                    }
                    return Ok(crate::workflow::OnFailure::Goto(target.trim().to_string()));
                }
                return Err(WorkflowParseError::TextError(
                    "on_failure.goto must be a string".into(),
                ));
            }
            Err(WorkflowParseError::TextError(
                "invalid on_failure mapping; expected { goto: <stage> }".into(),
            ))
        }
        _ => Err(WorkflowParseError::TextError(
            "invalid on_failure type; expected string or mapping".into(),
        )),
    }
}

pub fn parse_text_workflow(_src: &str) -> Result<WorkflowScript, WorkflowParseError> {
    // 预留为简单的以行或分隔符划定阶段的格式，如果需要的话可以扩展。目前暂不实现简单的文本解析，统一使用 YAML 解析
    Err(WorkflowParseError::TextError(
        "Text workflow parser is not yet implemented. Please use YAML format.".into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_yaml_workflow() {
        let yaml = r#"
stages:
  - name: parse
    expr: "from json"
  - name: filter
    expr: "where status == 'ok'"
"#;
        let script = parse_yaml_workflow(yaml).unwrap();
        assert_eq!(script.stages.len(), 2);
        assert_eq!(script.stages[0].name, "parse");
        assert_eq!(script.stages[0].expr.as_deref(), Some("from json"));
        assert_eq!(script.stages[1].name, "filter");
        assert_eq!(
            script.stages[1].expr.as_deref(),
            Some("where status == 'ok'")
        );
    }

    #[test]
    fn test_parse_yaml_on_failure_variants() {
        let yaml = r#"
stages:
  - name: s1
    expr: "from json"
    on_failure: "continue"
  - name: s2
    expr: "to json"
    on_failure: "goto(stage_end)"
  - name: stage_end
    expr: "to text"
    on_failure:
      goto: s1
"#;
        let script = parse_yaml_workflow(yaml).unwrap();
        assert!(matches!(
            script.stages[0].on_failure,
            crate::workflow::OnFailure::Continue
        ));
        match &script.stages[1].on_failure {
            crate::workflow::OnFailure::Goto(s) => assert_eq!(s, "stage_end"),
            _ => panic!("expected goto(stage_end)"),
        }
        match &script.stages[2].on_failure {
            crate::workflow::OnFailure::Goto(s) => assert_eq!(s, "s1"),
            _ => panic!("expected goto mapping"),
        }
    }
}
