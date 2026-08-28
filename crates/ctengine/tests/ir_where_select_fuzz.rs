#![cfg(feature = "feat_data_experimental")]

use ctdsl::parse;
use ctengine::context::{CommandRegistry, DataEngineContext};
use ctengine::eval_pipeline;
use ctpipeline::{CtPipelineData, CtPipelineMetadata, CtValue};
use rand::{Rng, SeedableRng};

fn compare_values(lhs: &CtValue, op: &str, rhs: i64) -> bool {
    let rhs = CtValue::Int(rhs);
    match op {
        "==" => lhs
            .as_int()
            .map(|v| v == rhs.as_int().unwrap())
            .unwrap_or(false),
        "!=" => lhs
            .as_int()
            .map(|v| v != rhs.as_int().unwrap())
            .unwrap_or(false),
        "<" => lhs
            .as_int()
            .map(|v| v < rhs.as_int().unwrap())
            .unwrap_or(false),
        ">" => lhs
            .as_int()
            .map(|v| v > rhs.as_int().unwrap())
            .unwrap_or(false),
        "<=" => lhs
            .as_int()
            .map(|v| v <= rhs.as_int().unwrap())
            .unwrap_or(false),
        ">=" => lhs
            .as_int()
            .map(|v| v >= rhs.as_int().unwrap())
            .unwrap_or(false),
        _ => false,
    }
}

fn ref_where_select(input: CtPipelineData, op: &str, rhs: i64) -> Result<CtPipelineData, String> {
    let meta = CtPipelineMetadata::default();
    match input {
        CtPipelineData::Value(CtValue::List(items), _) => {
            let mut out = Vec::new();
            for item in items {
                if let CtValue::Record(fields) = item {
                    let matched = fields
                        .iter()
                        .find(|(k, _)| k == "n")
                        .map(|(_, v)| compare_values(v, op, rhs))
                        .unwrap_or(false);
                    if matched {
                        let projected = fields
                            .iter()
                            .find(|(k, _)| k == "name")
                            .map(|(k, v)| vec![(k.clone(), v.clone())])
                            .unwrap_or_default();
                        out.push(CtValue::Record(projected));
                    }
                }
            }
            Ok(CtPipelineData::Value(CtValue::List(out), meta))
        }
        CtPipelineData::Value(CtValue::Record(fields), _) => {
            let matched = fields
                .iter()
                .find(|(k, _)| k == "n")
                .map(|(_, v)| compare_values(v, op, rhs))
                .unwrap_or(false);
            if !matched {
                return Err("select: empty input".to_string());
            }
            let projected = fields
                .iter()
                .find(|(k, _)| k == "name")
                .map(|(k, v)| vec![(k.clone(), v.clone())])
                .unwrap_or_default();
            Ok(CtPipelineData::Value(CtValue::Record(projected), meta))
        }
        CtPipelineData::Empty => Err("select: empty input".to_string()),
        _ => Err("where: expected Record or List input".to_string()),
    }
}

fn normalize(result: Result<CtPipelineData, String>) -> String {
    match result {
        Ok(data) => format!("ok:{data:?}"),
        Err(err) => format!("err:{err}"),
    }
}

#[test]
fn fuzz_ir_where_select_semantic_equivalence() {
    let mut rng = rand::rngs::StdRng::seed_from_u64(0x5EED_1178_6880);
    let ctx = DataEngineContext::new(CommandRegistry::empty(), None, None);
    let ops = ["==", "!=", "<", "<=", ">", ">="];

    for case in 0..512usize {
        let op = ops[rng.gen_range(0..ops.len())];
        let rhs = rng.gen_range(-128..=128);
        let expr = parse(&format!("where n {op} {rhs} | select name")).unwrap();

        let input = if rng.gen_bool(0.8) {
            let len = rng.gen_range(0..48usize);
            let mut items = Vec::new();
            for i in 0..len {
                let n = rng.gen_range(-256..=256);
                items.push(CtValue::Record(vec![
                    ("name".to_string(), CtValue::String(format!("c{case}_r{i}"))),
                    ("n".to_string(), CtValue::Int(n)),
                    ("aux".to_string(), CtValue::Int(n * 2)),
                ]));
                if rng.gen_bool(0.1) {
                    items.push(CtValue::Int(n));
                }
            }
            CtPipelineData::Value(CtValue::List(items), CtPipelineMetadata::default())
        } else {
            let n = rng.gen_range(-256..=256);
            CtPipelineData::Value(
                CtValue::Record(vec![
                    (
                        "name".to_string(),
                        CtValue::String(format!("single_{case}")),
                    ),
                    ("n".to_string(), CtValue::Int(n)),
                ]),
                CtPipelineMetadata::default(),
            )
        };

        let merged = eval_pipeline(&expr, input_to_owned(&input), &ctx).map_err(|e| e.to_string());
        let expected = ref_where_select(input, op, rhs);
        assert_eq!(
            normalize(merged),
            normalize(expected),
            "semantic mismatch at case={case}, op={op}, rhs={rhs}"
        );
    }
}

fn input_to_owned(input: &CtPipelineData) -> CtPipelineData {
    match input {
        CtPipelineData::Value(v, _) => {
            CtPipelineData::Value(v.clone(), CtPipelineMetadata::default())
        }
        CtPipelineData::Empty => CtPipelineData::Empty,
        _ => panic!("fuzz only uses Value/Empty inputs"),
    }
}
