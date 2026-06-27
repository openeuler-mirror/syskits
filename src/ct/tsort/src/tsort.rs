/*
 * Copyright(c) 2022-2024 China Telecom Cloud Technologies Co., Ltd. All rights reserved.
 *  syskits is licensed under Mulan PSL v2.
 * You can use this software according to the terms and conditions of the Mulan PSL V2
 * You may obtain a copy of Mulan PSL v2 at: http://license.coscl.org.cn/MulanPSL2
 * THIS SOFTWARE IS PROVIDED ON AN "AS IS" BASIS, WITHOUT WARRANTIES OF ANY
 * KIND, EITHER EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO
 * NON-INFRINGEMENT, MERCHANTABILITY OR FIT FOR A PARTICULAR PURPOSE.
 * See the Mulan PSL v2 for more details.
 */

//! tsort 命令行工具，用于对有依赖关系的项目进行拓扑排序

use clap::{Arg, Command, crate_version};
use ctcore::Tool;
use ctcore::ct_display::Quotable;
use ctcore::ct_error::{CTResult, CtSimpleError, FromIo};
use ctcore::{ct_format_usage, ct_help_about, ct_help_usage};
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::fs::File;
use std::io::{BufReader, Read, stdin};
use std::path::Path;

const TSORT_ABOUT: &str = ct_help_about!("tsort.md");
const TSORT_USAGE: &str = ct_help_usage!("tsort.md");

mod tsort_flags {
    pub const TSORT_FILE: &str = "file";
}

#[ctcore::main]
pub fn ctmain(args: impl ctcore::Args) -> CTResult<()> {
    tsort_main(args)
}

pub fn tsort_main(args: impl ctcore::Args) -> CTResult<()> {
    let matches = ct_app().try_get_matches_from(args)?;

    let input_file = matches
        .get_one::<String>(tsort_flags::TSORT_FILE)
        .expect("Value is required by clap");

    let mut stdin_buf;
    let mut file_buf;
    let mut buf_reader = BufReader::new(if input_file == "-" {
        stdin_buf = stdin();
        &mut stdin_buf as &mut dyn Read
    } else {
        let path = Path::new(&input_file);
        if path.is_dir() {
            return Err(CtSimpleError::new(
                1,
                format!("{}: read error: Is a directory", input_file),
            ));
        }
        file_buf = File::open(path).map_err_context(|| input_file.to_string())?;
        &mut file_buf as &mut dyn Read
    });

    let mut input_buffer = String::new();
    buf_reader.read_to_string(&mut input_buffer)?;
    let mut graph = TSortGraph::new();
    let mut all_tokens: Vec<_> = vec![];

    // 将所有输入行展开记录
    for buf_line in input_buffer.lines() {
        let mut tokens: Vec<_> = buf_line.split_whitespace().collect();
        if tokens.is_empty() {
            continue;
        } else {
            all_tokens.append(&mut tokens);
        }
    }
    // tokens按chunk处理数据
    for chunk in all_tokens.chunks(2) {
        match chunk.len() {
            2 => graph.add_edge(chunk[0], chunk[1]),
            _ => {
                let err_message = format!(
                    "{}: input contains an odd number of tokens",
                    input_file.maybe_quote()
                );
                return Err(CtSimpleError::new(1, err_message));
            }
        }
    }
    // 排序
    graph.tsort_exe();
    // 确认是否存在环
    if !graph.is_acyclic() {
        return Err(CtSimpleError::new(
            1,
            format!("{input_file}, input contains a loop:"),
        ));
    }

    for result in &graph.tsort_result {
        println!("{result}");
    }

    Ok(())
}

pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = TSORT_ABOUT;
    let usage_description = ct_format_usage(TSORT_USAGE);
    let arg = Arg::new(tsort_flags::TSORT_FILE)
        .default_value("-")
        .hide(true)
        .value_hint(clap::ValueHint::FilePath);

    Command::new(utility_name)
        .version(command_version)
        .about(application_info)
        .override_usage(usage_description)
        .infer_long_args(true)
        .arg(arg)
}

// 我们在这里使用String作为节点的表示形式
// 但是使用整数可能会提高性能。
#[derive(Default)]
struct TSortGraph<'input> {
    tsort_in_edges: BTreeMap<&'input str, BTreeSet<&'input str>>,
    tsort_out_edges: BTreeMap<&'input str, Vec<&'input str>>,
    tsort_result: Vec<&'input str>,
}

impl<'input> TSortGraph<'input> {
    fn new() -> Self {
        Self::default()
    }

    fn is_has_node(&self, n: &str) -> bool {
        self.tsort_in_edges.contains_key(n)
    }

    fn is_has_edge(&self, from: &str, to: &str) -> bool {
        self.tsort_in_edges[to].contains(from)
    }

    fn init_node(&mut self, n: &'input str) {
        self.tsort_in_edges.insert(n, BTreeSet::new());
        self.tsort_out_edges.insert(n, vec![]);
    }

    fn add_edge(&mut self, from: &'input str, to: &'input str) {
        if !self.is_has_node(to) {
            self.init_node(to);
        }

        if !self.is_has_node(from) {
            self.init_node(from);
        }

        if from != to && !self.is_has_edge(from, to) {
            self.tsort_in_edges.get_mut(to).unwrap().insert(from);
            self.tsort_out_edges.get_mut(from).unwrap().push(to);
        }
    }

    // 卡恩算法：
    // 初始化: 首先，找出所有没有前驱（即入度为0）的节点，并将它们放入一个队列中。入度是指指向该节点的边的数量。
    // 循环处理:从队列中取出一个节点，并将其添加到已排序的结果序列中。
    //        然后，遍历刚刚取出节点的所有后继节点（即该节点指向的节点），减少这些后继节点的入度计数。
    //        如果某个后继节点的入度变为0（意味着它现在也没有前驱了），则将它加入到队列中。
    // 检查是否有环: 当队列为空时，检查是否所有的节点都已经处理过。
    //             如果没有，说明原图中存在环，因为按照算法逻辑，所有无环的DAG都应该能够完成排序
    // O(|V|+|E|)
    fn tsort_exe(&mut self) {
        let mut start_nodes = vec![];
        for (n, btree_edges) in &self.tsort_in_edges {
            if btree_edges.is_empty() {
                start_nodes.push(*n);
            }
        }

        while !start_nodes.is_empty() {
            let n_str = start_nodes.remove(0);

            self.tsort_result.push(n_str);

            let n_out_edges_vec = self.tsort_out_edges.get_mut(&n_str).unwrap();
            #[allow(clippy::explicit_iter_loop)]
            for m in n_out_edges_vec.iter() {
                let m_in_edges_vec = self.tsort_in_edges.get_mut(m).unwrap();
                m_in_edges_vec.remove(&n_str);

                // 如果节点m没有其他in-coming edges，将其添加到起始start_nodes
                if m_in_edges_vec.is_empty() {
                    start_nodes.push(m);
                }
            }
            n_out_edges_vec.clear();
        }
    }

    fn is_acyclic(&self) -> bool {
        self.tsort_out_edges.values().all(|edge| edge.is_empty())
    }
}

#[derive(Default)]
pub struct Tsort;
impl Tool for Tsort {
    fn name(&self) -> &'static str {
        "tsort"
    }

    fn command(&self) -> Command {
        ct_app()
    }

    fn execute(&self, args: &[OsString]) -> CTResult<()> {
        tsort_main(args.iter().cloned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(test)]
    mod graph_tests {
        use super::*;

        #[test]
        fn test_is_has_node() {
            let mut graph = TSortGraph::new();
            graph.init_node("A");
            assert!(graph.is_has_node("A"));
            assert!(!graph.is_has_node("B"));
        }

        #[test]
        fn test_is_has_edge() {
            let mut graph = TSortGraph::new();
            graph.add_edge("A", "B");
            assert!(graph.is_has_edge("A", "B"));
            assert!(!graph.is_has_edge("B", "A"));
        }

        #[test]
        fn test_init_node() {
            let mut graph = TSortGraph::new();
            graph.init_node("A");
            assert!(graph.is_has_node("A"));
            assert!(graph.tsort_in_edges.get("A").is_some());
            assert!(graph.tsort_out_edges.get("A").is_some());
        }

        #[test]
        fn test_add_edge() {
            let mut graph = TSortGraph::new();
            graph.add_edge("A", "B");
            assert!(graph.is_has_node("A"));
            assert!(graph.is_has_node("B"));
            assert!(graph.is_has_edge("A", "B"));
        }

        #[test]
        fn test_tsort_exe() {
            let mut graph = TSortGraph::new();
            graph.add_edge("A", "B");
            graph.add_edge("B", "C");
            graph.add_edge("A", "C");
            graph.tsort_exe();
            assert_eq!(graph.tsort_result, vec!["A", "B", "C"]);
        }

        #[test]
        fn test_is_acyclic() {
            let mut graph = TSortGraph::new();
            graph.add_edge("A", "B");
            graph.add_edge("B", "C");
            graph.tsort_exe();
            assert!(graph.is_acyclic());

            let mut cyclic_graph = TSortGraph::new();
            cyclic_graph.add_edge("A", "B");
            cyclic_graph.add_edge("B", "C");
            cyclic_graph.add_edge("C", "A");
            cyclic_graph.tsort_exe();
            assert!(!cyclic_graph.is_acyclic());
        }
        #[test]
        fn test_tsort_with_multiple_start_nodes() {
            let mut graph = TSortGraph::new();
            graph.add_edge("A", "C");
            graph.add_edge("B", "C");
            graph.tsort_exe();
            assert!(
                graph.tsort_result == vec!["A", "B", "C"]
                    || graph.tsort_result == vec!["B", "A", "C"]
            );
        }

        #[test]
        fn test_tsort_with_no_edges() {
            let mut graph = TSortGraph::new();
            graph.init_node("A");
            graph.init_node("B");
            graph.init_node("C");
            graph.tsort_exe();
            assert!(graph.tsort_result.contains(&"A"));
            assert!(graph.tsort_result.contains(&"B"));
            assert!(graph.tsort_result.contains(&"C"));
            assert_eq!(graph.tsort_result.len(), 3);
        }

        #[test]
        fn test_tsort_with_single_node() {
            let mut graph = TSortGraph::new();
            graph.init_node("A");
            graph.tsort_exe();
            assert_eq!(graph.tsort_result, vec!["A"]);
        }

        #[test]
        fn test_tsort_with_duplicate_edges() {
            let mut graph = TSortGraph::new();
            graph.add_edge("A", "B");
            graph.add_edge("A", "B"); // duplicate edge
            graph.add_edge("B", "C");
            graph.tsort_exe();
            assert_eq!(graph.tsort_result, vec!["A", "B", "C"]);
        }

        #[test]
        fn test_tsort_with_self_loop() {
            let mut graph = TSortGraph::new();
            graph.add_edge("B", "A"); // self loop
            graph.add_edge("A", "B");
            graph.tsort_exe();
            assert!(!graph.is_acyclic());
        }

        #[test]
        fn test_add_edge_with_self_loop() {
            let mut graph = TSortGraph::new();
            graph.add_edge("A", "A");
            assert!(graph.is_has_node("A"));
            assert!(!graph.is_has_edge("A", "A")); // self loops are not added
        }
    }

    #[cfg(test)]
    mod ct_main_tests {
        use super::*;
        use std::ffi::OsString;
        use std::io::Write;
        use tempfile::tempdir;

        #[test]
        fn test_tsort_main_execution_default_nul_file() {
            let file_name = "test_tsort_main_execution_default_nul_file";

            let args = vec![ctcore::ct_util_name(), file_name];
            let result = tsort_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_err());
        }

        #[test]
        fn test_tsort_main_execution_default_file_no_data() {
            let dir = tempdir().unwrap();
            let file_path = dir
                .path()
                .join("test_tsort_main_execution_default_file_no_data");
            let _ = File::create(&file_path).unwrap();
            let file_name = file_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), file_name];
            let result = tsort_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_tsort_main_execution_default_file_data() {
            let dir = tempdir().unwrap();
            let file_path = dir
                .path()
                .join("test_tsort_main_execution_default_file_data");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(tmp_file, "a b c\nc d e\nf c g\nb c d").unwrap();
            let file_name = file_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), file_name];
            let result = tsort_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_ok());
        }

        #[test]
        fn test_tsort_main_execution_default_file_data_odd_number_err() {
            let dir = tempdir().unwrap();
            let file_path = dir
                .path()
                .join("test_tsort_main_execution_default_file_data");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(tmp_file, "a b c\nc d").unwrap();

            let file_name = file_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), file_name];
            let result = tsort_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_err());
            assert!(
                result
                    .unwrap_err()
                    .to_string()
                    .contains("input contains an odd number of tokens")
            );
        }

        #[test]
        fn test_tsort_main_execution_default_file_data_err() {
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("sort_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(tmp_file, "a b\nb a").unwrap();
            let file_name = file_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), file_name];
            let result = tsort_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_err());
        }

        #[test]
        fn test_tsort_main_execution_version() {
            let args_vec = vec![ctcore::ct_util_name(), "--version"];
            let args = args_vec.iter().map(|s| OsString::from(s));
            let result = tsort_main(args);

            assert!(result.is_err());
        }

        #[test]
        fn test_tsort_main_execution_other_version() {
            let args = vec![ctcore::ct_util_name(), "-V"];
            let result = tsort_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_err());
        }

        #[test]
        fn test_tsort_main_execution_help() {
            let args = vec![ctcore::ct_util_name(), "--help"];
            let result = tsort_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_err());
        }

        #[test]
        fn test_tsort_main_execution_help_short() {
            let args = vec![ctcore::ct_util_name(), "-h"];
            let result = tsort_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_err());
        }

        #[test]
        fn test_tsort_main_execution_unsupport_help() {
            let args = vec![ctcore::ct_util_name(), "-H"];
            let result = tsort_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_err());
        }

        #[test]
        fn test_tsort_main_invalid_argument() {
            let args = vec![ctcore::ct_util_name(), "--invalid-argument"];
            let result = tsort_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_err());
        }
    }

    #[cfg(test)]
    mod ct_app_tests {
        use clap::error::ErrorKind;

        use super::*;

        // tsort 接口: tsort [OPTIONS] FILE
        //
        // Options:
        //   -h, --help     Print help
        //   -V, --version  Print version

        #[test]
        fn test_ct_app_execution_version() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--version"];
            let executable = command.try_get_matches_from(args);

            assert!(executable.is_err());
            assert_eq!(executable.unwrap_err().kind(), ErrorKind::DisplayVersion);
        }

        #[test]
        fn test_ct_app_execution_other_version() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-V"];

            let executable = command.try_get_matches_from(args);

            assert!(executable.is_err());
            assert_eq!(executable.unwrap_err().kind(), ErrorKind::DisplayVersion);
        }

        #[test]
        fn test_ct_app_execution_help() {
            let command = ct_app();

            let help_args = vec![ctcore::ct_util_name(), "--help"];
            let result = command.try_get_matches_from(help_args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayHelp);
        }

        #[test]
        fn test_ct_app_execution_help_short() {
            let command = ct_app();

            let help_args = vec![ctcore::ct_util_name(), "-h"];
            let result = command.try_get_matches_from(help_args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayHelp);
        }

        #[test]
        fn test_ct_app_execution_unsupport_help() {
            let command = ct_app();

            let help_args = vec![ctcore::ct_util_name(), "-H"];
            let result = command.try_get_matches_from(help_args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::UnknownArgument);
        }

        #[test]
        fn test_ct_app_invalid_argument() {
            let command = ct_app();

            let invalid_args = vec![ctcore::ct_util_name(), "--invalid-argument"];
            let result = command.try_get_matches_from(invalid_args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::UnknownArgument);
        }

        #[test]
        fn test_ct_app_support_missing_argument() {
            let command = ct_app();

            let missing_args = vec![ctcore::ct_util_name()]; // 缺少任何参数
            let result = command.try_get_matches_from(missing_args);
            assert!(result.is_ok());
        }
    }
}
