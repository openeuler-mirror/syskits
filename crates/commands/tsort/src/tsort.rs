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

//! tsort 命令行工具，用于对有依赖关系的项目进行拓扑排序

extern crate rust_i18n;
use clap::{Arg, Command, crate_version};
use rust_i18n::t;
rust_i18n::i18n!("locales", fallback = "en-US");
use ctcore::Tool;
use ctcore::ct_display::Quotable;
use ctcore::ct_error::{CTResult, CtSimpleError, FromIo};
use ctcore::ct_show_error;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::ffi::OsString;
use std::fs::File;
use std::io::{BufReader, Read, stdin};
use std::path::Path;
use sys_locale::get_locale;
mod tsort_flags {
    pub const TSORT_FILE: &str = "file";
}

pub fn tsort_main(args: impl ctcore::Args) -> CTResult<()> {
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
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
                format!("{input_file}: read error: Is a directory"),
            ));
        }
        file_buf = File::open(path).map_err_context(|| input_file.to_string())?;
        &mut file_buf as &mut dyn Read
    });

    let mut input_buffer = String::new();
    buf_reader.read_to_string(&mut input_buffer)?;
    let mut graph = TSortGraph::new();

    let mut tokens = Vec::new();
    for buf_line in input_buffer.lines() {
        for tok in buf_line.split_whitespace() {
            if !tok.is_empty() {
                tokens.push(tok.to_string());
            }
        }
    }

    if tokens.len() % 2 != 0 {
         let err_message = format!(
            "{}: input contains an odd number of tokens",
            input_file.maybe_quote()
        );
        return Err(CtSimpleError::new(1, err_message));
    }

    for chunk in tokens.chunks(2) {
        graph.add_edge(chunk[0].clone(), chunk[1].clone());
    }

    let exit_code = graph.tsort_exe(input_file);

    if exit_code != 0 {
        return Err(CtSimpleError::new(1, ""));
    }

    Ok(())
}

pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = t!("tsort.about");
    let usage_description = t!("tsort.usage");
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

use std::collections::VecDeque;

#[derive(Default)]
struct TSortGraph {
    in_edges: BTreeMap<String, BTreeSet<String>>,
    out_edges: BTreeMap<String, Vec<String>>,
}

impl TSortGraph {
    fn new() -> Self {
        Self::default()
    }

    fn init_node(&mut self, n: String) {
        self.in_edges.entry(n.clone()).or_insert_with(BTreeSet::new);
        self.out_edges.entry(n).or_insert_with(Vec::new);
    }

    fn add_edge(&mut self, from: String, to: String) {
        self.init_node(from.clone());
        self.init_node(to.clone());

        if from != to {
            let in_set = self.in_edges.get_mut(&to).unwrap();
            if !in_set.contains(&from) {
                in_set.insert(from.clone());
                self.out_edges.get_mut(&from).unwrap().push(to);
            }
        }
    }

    fn tsort_exe(&mut self, filename: &str) -> i32 {
        let mut found_cycle = false;
        let mut queue: VecDeque<String> = VecDeque::new();

        // 1. 初始化扫描 (GNU walk_tree scan_zeros)
        // BTreeMap 默认迭代顺序是 key 的升序 (Alphabetical)，这与 C 语言 walk_tree 一致
        for (node, preds) in &self.in_edges {
            if preds.is_empty() {
                queue.push_back(node.clone());
            }
        }

        while !self.in_edges.is_empty() {
            if queue.is_empty() {
                found_cycle = true;
                if let Some(freed_node) = self.detect_and_break_cycle(filename) {
                    if let Some(preds) = self.in_edges.get(&freed_node) {
                        if preds.is_empty() {
                            queue.push_back(freed_node);
                        }
                    }
                } else {
                    break; 
                }
            }

            while let Some(n) = queue.pop_front() {
                println!("{}", n);

                if let Some(succs) = self.out_edges.remove(&n) {
                    // 模拟 C 语言 successor 链表的 LIFO 行为，必须反向遍历
                    for succ in succs.into_iter().rev() {
                        if let Some(preds) = self.in_edges.get_mut(&succ) {
                            preds.remove(&n);
                            if preds.is_empty() {
                                queue.push_back(succ);
                            }
                        }
                    }
                }
                self.in_edges.remove(&n);
            }
        }

        if found_cycle { 1 } else { 0 }
    }

    // 完全重写：模拟 GNU tsort 的反向搜索 (Reverse Search) 算法
    fn detect_and_break_cycle(&mut self, filename: &str) -> Option<String> {
        let candidates: Vec<String> = self.in_edges.keys().cloned().collect();
        
        // 对应 GNU 代码中的 static struct item *loop
        let mut cursor: Option<String> = None;
        
        // 对应 GNU 代码中的 qlink (记录路径: key -> value 表示 key 指向 value)
        // 注意 GNU 的 qlink 是反向链表，但在 detect_loop 中它的构建方式是：
        // 找到 k 指向 loop，则 k->qlink = loop。所以这是正向的路径记录 (Predecessor -> Successor)
        let mut qlink: HashMap<String, String> = HashMap::new();

        // 模拟 GNU 的 repeated walk_tree
        loop {
            // 每次都从头遍历所有节点 (A..Z)
            for k in &candidates {
                // 如果当前没有 cursor，找第一个有入度的节点作为起点
                if cursor.is_none() {
                    // 检查 k 是否有入度 (count > 0)
                    if self.in_edges.get(k).map(|s| !s.is_empty()).unwrap_or(false) {
                        cursor = Some(k.clone());
                    }
                    continue;
                }

                let curr = cursor.as_ref().unwrap();

                // 核心逻辑：检查 k 是否指向 curr (即 k 是 curr 的前驱)
                // GNU: if ((*p)->suc == loop)
                let k_points_to_curr = self.out_edges.get(k)
                    .map(|succs| succs.contains(curr))
                    .unwrap_or(false);

                if k_points_to_curr {
                    // 检查 k 是否已经在当前路径中 (GNU: if (k->qlink))
                    if qlink.contains_key(k) {
                        // *** 发现了环 ***
                        ct_show_error!("{}: input contains a loop:", filename.maybe_quote());

                        // 回溯打印环 (GNU: while (loop) ... until loop == k)
                        // 我们当前的 cursor 就是 GNU 的 loop
                        let mut loop_node = curr.clone();
                        
                        // 1. 打印 loop_node
                        ct_show_error!("{}", loop_node);

                        // 开始回溯
                        loop {
                            // 获取路径上的下一个节点
                            let next_node = qlink.get(&loop_node).unwrap().clone();
                            
                            // 打印下一个节点 (但如果是 k 就不打印了，因为 k 在循环外已经被找到了)
                            // 仔细看 GNU 逻辑：
                            // print loop->str
                            // if loop == k: break
                            // loop = loop->qlink
                            
                            if loop_node == *k {
                                // 此时 loop_node 就是 k。
                                // GNU 在这里移除 relation: s = *p (即 k -> curr 的边)
                                if let Some(preds) = self.in_edges.get_mut(curr) {
                                    preds.remove(k);
                                }
                                if let Some(succs) = self.out_edges.get_mut(k) {
                                    if let Some(pos) = succs.iter().position(|x| x == curr) {
                                        succs.remove(pos);
                                    }
                                }
                                // 返回被释放入度的节点 (curr)
                                return Some(curr.clone());
                            }
                            
                            ct_show_error!("{}", next_node);
                            loop_node = next_node;
                        }
                    } else {
                        // 记录路径：k 指向 curr
                        qlink.insert(k.clone(), curr.clone());
                        // 移动 cursor 到 k
                        cursor = Some(k.clone());
                    }
                }
            }
            
            // 如果遍历了一整圈都没找到 cursor，说明逻辑结束 (不应该发生，除非图空了)
            if cursor.is_none() {
                return None;
            }
        }
    }

    fn find_cycle_dfs(
        &self,
        curr: &String,
        path: &mut Vec<String>,
        visited_in_path: &mut HashSet<String>,
        break_edge: &mut Option<(String, String)>,
    ) -> bool {
        path.push(curr.clone());
        visited_in_path.insert(curr.clone());

        if let Some(succs) = self.out_edges.get(curr) {
            // 这里同样使用 LIFO (rev) 顺序，以匹配 GNU 在 detect_loop 中遍历 successor 链表的顺序
            for next in succs.iter().rev() {
                if visited_in_path.contains(next) {
                    *break_edge = Some((curr.clone(), next.clone()));
                    return true;
                }
                if self.find_cycle_dfs(next, path, visited_in_path, break_edge) {
                    return true;
                }
            }
        }

        visited_in_path.remove(curr);
        path.pop();
        false
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
    use std::ffi::OsString;

    #[test]
    fn test_tool_implementation() {
        let tool = Tsort;

        // 测试 name 方法
        assert_eq!(tool.name(), "tsort");

        // 测试 command 方法
        let command = tool.command();
        assert!(command.get_name().contains("tsort"));

        // 测试 execute 方法
        let args = vec![OsString::from("tsort"), OsString::from("--help")];
        assert!(tool.execute(&args).is_err());
    }

    #[cfg(test)]
    mod graph_tests {
        use super::*;

        // 辅助函数：简化 String 的创建
        fn s(text: &str) -> String {
            text.to_string()
        }

        #[test]
        fn test_node_existence() {
            let mut graph = TSortGraph::new();
            graph.init_node(s("A"));
            
            // 直接检查内部 BTreeMap 是否包含键
            assert!(graph.in_edges.contains_key("A"));
            assert!(!graph.in_edges.contains_key("B"));
            
            // 检查出度表是否初始化
            assert!(graph.out_edges.contains_key("A"));
        }

        #[test]
        fn test_edge_existence() {
            let mut graph = TSortGraph::new();
            graph.add_edge(s("A"), s("B"));

            // 验证节点存在
            assert!(graph.in_edges.contains_key("A"));
            assert!(graph.in_edges.contains_key("B"));

            // 验证边存在 (A -> B)
            // A 的出度包含 B
            assert!(graph.out_edges.get("A").unwrap().contains(&s("B")));
            // B 的入度包含 A
            assert!(graph.in_edges.get("B").unwrap().contains(&s("A")));

            // 验证反向边不存在
            assert!(!graph.out_edges.get("B").unwrap().contains(&s("A")));
        }

        #[test]
        fn test_init_node() {
            let mut graph = TSortGraph::new();
            graph.init_node(s("A"));
            
            assert!(graph.in_edges.contains_key("A"));
            assert!(graph.in_edges.get("A").unwrap().is_empty()); // 新初始化的节点入度为0
            assert!(graph.out_edges.get("A").unwrap().is_empty());
        }

        #[test]
        fn test_tsort_execution_success() {
            let mut graph = TSortGraph::new();
            // A -> B -> C
            // A -> C
            graph.add_edge(s("A"), s("B"));
            graph.add_edge(s("B"), s("C"));
            graph.add_edge(s("A"), s("C"));

            // 执行排序
            // 注意：tsort_exe 会打印到 stdout，单元测试通常无法捕获 stdout 内容。
            // 我们主要验证：1. 返回码为 0; 2. 图被“消耗”殆尽（所有节点都被处理并移除）。
            let exit_code = graph.tsort_exe("test");
            
            assert_eq!(exit_code, 0);
            assert!(graph.in_edges.is_empty(), "Graph should be empty after successful sort");
        }

        #[test]
        fn test_cycle_detection() {
            let mut graph = TSortGraph::new();
            // A -> B -> C -> A (环)
            graph.add_edge(s("A"), s("B"));
            graph.add_edge(s("B"), s("C"));
            graph.add_edge(s("C"), s("A"));

            // 执行排序
            // GNU 逻辑：遇到环会报错（打印到 stderr），破环，然后继续。
            // 最终因为发现了环，返回值应该是 1。
            let exit_code = graph.tsort_exe("test");
            
            assert_eq!(exit_code, 1);
            
            // 即便有环，现在的逻辑也会打破它并输出所有节点，所以最终图也应该是空的
            assert!(graph.in_edges.is_empty());
        }

        #[test]
        fn test_tsort_with_multiple_start_nodes() {
            let mut graph = TSortGraph::new();
            // A -> C, B -> C
            graph.add_edge(s("A"), s("C"));
            graph.add_edge(s("B"), s("C"));

            let exit_code = graph.tsort_exe("test");
            assert_eq!(exit_code, 0);
            assert!(graph.in_edges.is_empty());
        }

        #[test]
        fn test_tsort_with_no_edges() {
            let mut graph = TSortGraph::new();
            // 只有独立节点
            graph.init_node(s("A"));
            graph.init_node(s("B"));
            graph.init_node(s("C"));

            let exit_code = graph.tsort_exe("test");
            assert_eq!(exit_code, 0);
            assert!(graph.in_edges.is_empty());
        }

        #[test]
        fn test_tsort_with_single_node() {
            let mut graph = TSortGraph::new();
            graph.init_node(s("A"));

            let exit_code = graph.tsort_exe("test");
            assert_eq!(exit_code, 0);
            assert!(graph.in_edges.is_empty());
        }

        #[test]
        fn test_tsort_with_duplicate_edges() {
            let mut graph = TSortGraph::new();
            graph.add_edge(s("A"), s("B"));
            graph.add_edge(s("A"), s("B")); // 重复添加
            graph.add_edge(s("B"), s("C"));

            // 验证内部状态：A 的出度应该只有 1 个 B，不应该重复
            assert_eq!(graph.out_edges.get("A").unwrap().len(), 1);
            assert_eq!(graph.in_edges.get("B").unwrap().len(), 1);

            let exit_code = graph.tsort_exe("test");
            assert_eq!(exit_code, 0);
        }

        #[test]
        fn test_self_loop_behavior() {
            // 对应输入: A A
            // GNU tsort 逻辑：如果 from == to，则忽略边的关系，但记录节点。
            // 所以这不算“环”（Cycle），而是普通节点。
            let mut graph = TSortGraph::new();
            graph.add_edge(s("A"), s("A"));

            // 验证：节点存在，但没有自环边
            assert!(graph.in_edges.contains_key("A"));
            assert!(graph.in_edges.get("A").unwrap().is_empty());
            
            let exit_code = graph.tsort_exe("test");
            assert_eq!(exit_code, 0); // 自环不视为错误
        }

        #[test]
        fn test_explicit_cycle_two_nodes() {
            // 对应输入: A B, B A
            let mut graph = TSortGraph::new();
            graph.add_edge(s("A"), s("B"));
            graph.add_edge(s("B"), s("A"));

            let exit_code = graph.tsort_exe("test");
            assert_eq!(exit_code, 1); // 存在真正的环
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

            let args = [ctcore::ct_util_name(), file_name];
            let result = tsort_main(args.iter().map(OsString::from));

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

            let args = [ctcore::ct_util_name(), file_name];
            let result = tsort_main(args.iter().map(OsString::from));

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

            let args = [ctcore::ct_util_name(), file_name];
            let result = tsort_main(args.iter().map(OsString::from));
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

            let args = [ctcore::ct_util_name(), file_name];
            let result = tsort_main(args.iter().map(OsString::from));
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

            let args = [ctcore::ct_util_name(), file_name];
            let result = tsort_main(args.iter().map(OsString::from));

            assert!(result.is_err());
        }

        #[test]
        fn test_tsort_main_execution_version() {
            let args_vec = [ctcore::ct_util_name(), "--version"];
            let args = args_vec.iter().map(OsString::from);
            let result = tsort_main(args);

            assert!(result.is_err());
        }

        #[test]
        fn test_tsort_main_execution_other_version() {
            let args = [ctcore::ct_util_name(), "-V"];
            let result = tsort_main(args.iter().map(OsString::from));

            assert!(result.is_err());
        }

        #[test]
        fn test_tsort_main_execution_help() {
            let args = [ctcore::ct_util_name(), "--help"];
            let result = tsort_main(args.iter().map(OsString::from));
            assert!(result.is_err());
        }

        #[test]
        fn test_tsort_main_execution_help_short() {
            let args = [ctcore::ct_util_name(), "-h"];
            let result = tsort_main(args.iter().map(OsString::from));
            assert!(result.is_err());
        }

        #[test]
        fn test_tsort_main_execution_unsupport_help() {
            let args = [ctcore::ct_util_name(), "-H"];
            let result = tsort_main(args.iter().map(OsString::from));
            assert!(result.is_err());
        }

        #[test]
        fn test_tsort_main_invalid_argument() {
            let args = [ctcore::ct_util_name(), "--invalid-argument"];
            let result = tsort_main(args.iter().map(OsString::from));
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
