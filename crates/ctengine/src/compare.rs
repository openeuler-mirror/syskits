use ctpipeline::CtValue;

pub fn resolve_field_path<'a>(
    fields: &'a [(String, CtValue)],
    field_path: &str,
) -> Option<&'a CtValue> {
    let normalized = field_path
        .strip_prefix("$it.")
        .or_else(|| field_path.strip_prefix("it."))
        .unwrap_or(field_path);
    let mut parts = normalized.split('.');
    let first = parts.next()?;
    let mut current = fields.iter().find(|(k, _)| k == first).map(|(_, v)| v)?;
    for part in parts {
        current = match current {
            CtValue::Record(inner) => inner.iter().find(|(k, _)| k == part).map(|(_, v)| v)?,
            _ => return None,
        };
    }
    Some(current)
}

pub fn compare_values(lhs: &CtValue, op: &str, rhs: &CtValue) -> bool {
    match op {
        "==" => ct_eq(lhs, rhs),
        "!=" => !ct_eq(lhs, rhs),
        "<" => ct_cmp(lhs, rhs).map(|o| o.is_lt()).unwrap_or(false),
        ">" => ct_cmp(lhs, rhs).map(|o| o.is_gt()).unwrap_or(false),
        "<=" => ct_cmp(lhs, rhs).map(|o| o.is_le()).unwrap_or(false),
        ">=" => ct_cmp(lhs, rhs).map(|o| o.is_ge()).unwrap_or(false),
        _ => false,
    }
}

pub fn ct_eq(a: &CtValue, b: &CtValue) -> bool {
    match (a, b) {
        (CtValue::Int(x), CtValue::Int(y)) => x == y,
        (CtValue::Float(x), CtValue::Float(y)) => x == y,
        (CtValue::String(x), CtValue::String(y)) => x == y,
        (CtValue::Bool(x), CtValue::Bool(y)) => x == y,
        (CtValue::Nothing, CtValue::Nothing) => true,
        (CtValue::DateTime(x), CtValue::DateTime(y)) => x == y,
        (CtValue::Duration(x), CtValue::Duration(y)) => x == y,
        (CtValue::Size(x), CtValue::Size(y)) => x == y,
        // Int/Float 跨类型相等
        (CtValue::Int(i), CtValue::Float(f)) | (CtValue::Float(f), CtValue::Int(i)) => {
            (*i as f64) == *f
        }
        // Int/Size 跨类型相等（正整数可比较）
        (CtValue::Int(i), CtValue::Size(s)) | (CtValue::Size(s), CtValue::Int(i)) => {
            *i >= 0 && *i as u64 == *s
        }
        _ => false,
    }
}

pub fn ct_cmp(a: &CtValue, b: &CtValue) -> Option<std::cmp::Ordering> {
    match (a, b) {
        // Nothing 的排序策略由上层显式决定（例如 sort-by 的 nulls-first/last）。
        // 在通用比较器里，仅定义 Nothing 与 Nothing 的相等关系。
        (CtValue::Nothing, CtValue::Nothing) => Some(std::cmp::Ordering::Equal),
        (CtValue::Nothing, _) => None,
        (_, CtValue::Nothing) => None,
        (CtValue::Int(x), CtValue::Int(y)) => Some(x.cmp(y)),
        (CtValue::Float(x), CtValue::Float(y)) => x.partial_cmp(y),
        (CtValue::String(x), CtValue::String(y)) => Some(x.cmp(y)),
        (CtValue::DateTime(x), CtValue::DateTime(y)) => Some(x.cmp(y)),
        (CtValue::Duration(x), CtValue::Duration(y)) => Some(x.cmp(y)),
        (CtValue::Size(x), CtValue::Size(y)) => Some(x.cmp(y)),
        // Int/Float 跨类型有序比较
        (CtValue::Int(i), CtValue::Float(f)) => (*i as f64).partial_cmp(f),
        (CtValue::Float(f), CtValue::Int(i)) => f.partial_cmp(&(*i as f64)),
        // Int/Size 跨类型有序比较
        (CtValue::Int(i), CtValue::Size(s)) => {
            if *i < 0 {
                Some(std::cmp::Ordering::Less)
            } else {
                Some((*i as u64).cmp(s))
            }
        }
        (CtValue::Size(s), CtValue::Int(i)) => {
            if *i < 0 {
                Some(std::cmp::Ordering::Greater)
            } else {
                Some(s.cmp(&(*i as u64)))
            }
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_nested_path() {
        let fields = vec![(
            "a".to_string(),
            CtValue::Record(vec![("b".to_string(), CtValue::Int(42))]),
        )];
        let v = resolve_field_path(&fields, "$it.a.b").unwrap();
        assert!(matches!(v, CtValue::Int(42)));
    }

    #[test]
    fn compare_scalar_values() {
        assert!(compare_values(&CtValue::Int(2), ">", &CtValue::Int(1)));
        assert!(compare_values(
            &CtValue::String("a".into()),
            "==",
            &CtValue::String("a".into())
        ));
        assert!(!compare_values(
            &CtValue::Bool(true),
            "<",
            &CtValue::Bool(false)
        ));
    }

    #[test]
    fn compare_datetime_values() {
        let dt1 = CtValue::DateTime(1_000_000_000);
        let dt2 = CtValue::DateTime(2_000_000_000);
        assert!(compare_values(&dt1, "<", &dt2));
        assert!(compare_values(&dt2, ">", &dt1));
        assert!(compare_values(&dt1, "==", &dt1));
        assert!(compare_values(&dt1, "!=", &dt2));
    }

    #[test]
    fn compare_duration_values() {
        let d1 = CtValue::Duration(60_000_000_000);
        let d2 = CtValue::Duration(120_000_000_000);
        assert!(compare_values(&d1, "<", &d2));
        assert!(compare_values(&d1, "<=", &d2));
    }

    #[test]
    fn compare_size_values() {
        let s1 = CtValue::Size(1024);
        let s2 = CtValue::Size(1024 * 1024);
        assert!(compare_values(&s1, "<", &s2));
        assert!(compare_values(&s2, ">=", &s1));
        assert!(compare_values(&s1, "==", &s1));
    }

    #[test]
    fn compare_int_float_cross_type() {
        assert!(compare_values(&CtValue::Int(2), "==", &CtValue::Float(2.0)));
        assert!(compare_values(&CtValue::Int(1), "<", &CtValue::Float(1.5)));
        assert!(compare_values(&CtValue::Float(3.0), ">", &CtValue::Int(2)));
    }

    #[test]
    fn compare_int_size_cross_type() {
        assert!(compare_values(
            &CtValue::Int(1024),
            "==",
            &CtValue::Size(1024)
        ));
        assert!(compare_values(&CtValue::Int(-1), "<", &CtValue::Size(0)));
        assert!(compare_values(&CtValue::Size(100), ">", &CtValue::Int(50)));
    }

    #[test]
    fn compare_nothing_ordering() {
        assert_eq!(ct_cmp(&CtValue::Nothing, &CtValue::Int(1)), None);
        assert_eq!(ct_cmp(&CtValue::Int(1), &CtValue::Nothing), None);
        assert_eq!(
            ct_cmp(&CtValue::Nothing, &CtValue::Nothing),
            Some(std::cmp::Ordering::Equal)
        );
        assert!(!compare_values(
            &CtValue::Nothing,
            "<",
            &CtValue::String("x".into())
        ));
        assert!(!compare_values(
            &CtValue::Nothing,
            ">",
            &CtValue::String("x".into())
        ));
        assert!(compare_values(&CtValue::Nothing, "==", &CtValue::Nothing));
    }
}
