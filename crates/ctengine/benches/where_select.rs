#![cfg(feature = "feat_data_experimental")]

use ctdsl::parse;
use ctengine::context::{CommandRegistry, DataEngineContext};
use ctengine::eval_pipeline;
use ctpipeline::{CtPipelineData, CtPipelineMetadata, CtValue};
use std::time::Instant;

fn make_input(rows: usize) -> CtPipelineData {
    let mut items = Vec::with_capacity(rows);
    for i in 0..rows {
        let n = (i as i64) % 1024 - 512;
        items.push(CtValue::Record(vec![
            ("name".to_string(), CtValue::String(format!("row_{i}"))),
            ("n".to_string(), CtValue::Int(n)),
            ("payload".to_string(), CtValue::Int(i as i64)),
        ]));
    }
    CtPipelineData::Value(CtValue::List(items), CtPipelineMetadata::default())
}

fn ref_where_select(input: CtPipelineData, rhs: i64) -> CtPipelineData {
    let meta = CtPipelineMetadata::default();
    let CtPipelineData::Value(CtValue::List(items), _) = input else {
        return CtPipelineData::Empty;
    };

    let mut out = Vec::new();
    for item in items {
        let CtValue::Record(fields) = item else {
            continue;
        };
        let matched = fields
            .iter()
            .find(|(k, _)| k == "n")
            .and_then(|(_, v)| v.as_int().ok())
            .map(|n| n >= rhs)
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
    CtPipelineData::Value(CtValue::List(out), meta)
}

fn main() {
    let expr = parse("where n >= 0 | select name").expect("parse benchmark expr");
    let ctx = DataEngineContext::new(CommandRegistry::empty(), None, None);
    let rounds = 30usize;
    let rows = 50_000usize;
    let rhs = 0i64;

    let mut merged_ms = 0u128;
    let mut sequential_ms = 0u128;

    for _ in 0..rounds {
        let t0 = Instant::now();
        let _ = eval_pipeline(&expr, make_input(rows), &ctx).expect("merged run failed");
        merged_ms += t0.elapsed().as_millis();

        let t1 = Instant::now();
        let _ = ref_where_select(make_input(rows), rhs);
        sequential_ms += t1.elapsed().as_millis();
    }

    println!(
        "bench(where+select): rounds={rounds}, rows={rows}, merged={}ms, sequential={}ms",
        merged_ms, sequential_ms
    );
}
