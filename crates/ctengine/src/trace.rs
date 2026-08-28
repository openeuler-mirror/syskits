/*
 * Copyright(c) 2022-2025 China Telecom Cloud Technologies Co., Ltd. All rights reserved.
 *  syskits is licensed under Mulan PSL v2.
 * You can use this software according to the terms and conditions of the Mulan PSL V2.
 * You may obtain a copy of Mulan PSL v2 at: http://license.coscl.org.cn/MulanPSL2.
 * THIS SOFTWARE IS PROVIDED ON AN "AS IS" BASIS, WITHOUT WARRANTIES OF ANY
 * KIND, EITHER EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO
 * NON-INFRINGEMENT, MERCHANTABILITY OR FIT FOR A PARTICULAR PURPOSE.
 * See the Mulan PSL v2 for more details.
 */

//! Pipeline trace data model.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TraceStatus {
    Ok,
    Error(String),
    Skipped,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StageTrace {
    pub name: Option<String>,
    pub cmd: String,
    pub duration_ms: u64,
    pub rows_in: usize,
    pub rows_out: usize,
    pub status: TraceStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PipelineTrace {
    pub stages: Vec<StageTrace>,
    pub total_ms: u64,
}

impl PipelineTrace {
    pub fn reset(&mut self) {
        self.stages.clear();
        self.total_ms = 0;
    }

    pub fn record(&mut self, stage: StageTrace) {
        self.stages.push(stage);
    }

    pub fn set_total_ms(&mut self, total_ms: u64) {
        self.total_ms = total_ms;
    }

    pub fn format_lines(&self) -> Vec<String> {
        let mut lines = Vec::with_capacity(self.stages.len() + 1);
        lines.push(format!(
            "[trace] pipeline: {} stages, {}ms total",
            self.stages.len(),
            self.total_ms
        ));
        for (idx, stage) in self.stages.iter().enumerate() {
            let mut line = format!(
                "  [trace] stage[{idx}] {}: {}ms, {} in, {} out",
                stage.cmd, stage.duration_ms, stage.rows_in, stage.rows_out
            );
            match &stage.status {
                TraceStatus::Ok => {}
                TraceStatus::Error(msg) => line.push_str(&format!(", error: {msg}")),
                TraceStatus::Skipped => line.push_str(", skipped"),
            }
            lines.push(line);
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_trace_format_lines() {
        let mut t = PipelineTrace::default();
        t.record(StageTrace {
            name: None,
            cmd: "from".into(),
            duration_ms: 2,
            rows_in: 0,
            rows_out: 1,
            status: TraceStatus::Ok,
        });
        t.set_total_ms(2);
        let lines = t.format_lines();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("1 stages"));
        assert!(lines[1].contains("stage[0] from: 2ms, 0 in, 1 out"));
    }

    #[test]
    fn test_trace_reset_clears_existing_data() {
        let mut t = PipelineTrace::default();
        t.record(StageTrace {
            name: Some("demo".into()),
            cmd: "from".into(),
            duration_ms: 1,
            rows_in: 0,
            rows_out: 1,
            status: TraceStatus::Ok,
        });
        t.set_total_ms(42);
        t.reset();
        assert!(t.stages.is_empty());
        assert_eq!(t.total_ms, 0);
    }
}
