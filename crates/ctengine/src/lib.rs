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

//! `ctengine` — syskits 数据管线执行引擎（M1a 骨架，M1b Direct Interpreter）。
//!
//! 提供：
//! - [`command`]：`DataCommand` trait、`DataCommandFactory` 类型别名
//! - [`context`]：`DataEngineContext`、`OutDest`、`ErrDest`、`SignalHandle`、`CommandRegistry`
//! - [`error`]：`CtDiagnosticError`（实现 `CTError`）
//! - [`interpreter`]：`eval_pipeline()`（Direct Interpreter 执行循环）
//! - [`entry`]：`run_data_entry`（`syskits data` 的顶层入口）

pub mod adapters;
pub mod command;
pub mod compare;
pub mod context;
pub(crate) mod display;
pub mod entry;
pub mod error;
pub mod execution;
pub mod external;
pub mod reusable_input;
pub mod trace;
pub mod workflow_vars;
pub use external::*;
pub mod interpreter;
pub mod legacy_adapter;
pub mod pipeline_stdin;

#[cfg(feature = "workflow")]
pub mod workflow;
#[cfg(feature = "workflow")]
pub mod workflow_parser;

#[cfg(feature = "workflow")]
pub use workflow::{WorkflowScript, WorkflowStage, run_workflow};

pub use adapters::{DataAdapter, LegacyAdapter, legacy_default_output_profile};
pub use command::{DataCommand, DataCommandFactory};
pub use context::{CommandRegistry, DataEngineContext, EnvStore, ErrDest, OutDest, SignalHandle};
pub use entry::{
    eval_expr, parse_and_eval_expr, run_data_entry, run_data_entry_with_registry,
    run_data_entry_with_registry_and_legacy,
};
pub use error::CtDiagnosticError;
pub use execution::{
    CommandCore, CommandRunner, ExitPolicy, OutputFormat, OutputProfile, exit_code,
};
#[allow(deprecated)]
pub use interpreter::{eval_pipeline, print_pipeline_data, try_print_pipeline_data};
pub use legacy_adapter::{LegacyToolAdapter, LegacyToolResolver};
pub use pipeline_stdin::{
    argv_has_stdin_operand, argv_uses_stdin, run_with_optional_pipeline_stdin,
    run_with_optional_pipeline_stdin_io, write_pipeline_as_text,
};
pub use reusable_input::ReusableInput;
pub use trace::{PipelineTrace, StageTrace, TraceStatus};
pub mod binder;
pub mod ir;
pub mod ir_compiler;
pub mod nir;
