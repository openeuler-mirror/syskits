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
use ctcore::Tool;
use ctcore::ct_display::Quotable;
use ctcore::ct_error::{CTResult, set_ct_exit_code, strip_errno};
use rust_i18n::t;
rust_i18n::i18n!("locales", fallback = "en-US");
use std::collections::{BTreeMap, HashMap};
use std::ffi::OsString;
use std::fs::File;
use std::io::{Read, Write};
use std::ops::Range;
use std::path::Path;
use sys_locale::get_locale;

mod tsort_flags {
    pub const TSORT_FILE: &str = "file";
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TsortRow {
    pub row_index: usize,
    pub node: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TsortSemantic {
    pub had_cycle: bool,
    pub rows: Vec<TsortRow>,
    pub classic_text: String,
    pub classic_bytes: Vec<u8>,
    pub stderr_text: String,
    pub exit_code: i32,
}

struct TsortRunOutcome {
    had_cycle: bool,
    rows: Vec<TsortRow>,
    classic_bytes: Vec<u8>,
    stderr_bytes: Vec<u8>,
    output_events: Vec<TsortOutputEvent>,
    exit_code: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum TsortOutputEvent {
    Stdout(Range<usize>),
    Stderr(Range<usize>),
}

pub fn tsort_main(args: impl ctcore::Args) -> CTResult<()> {
    let outcome = tsort_run(args)?;
    let stdout = std::io::stdout();
    let stderr = std::io::stderr();
    let mut stdout = stdout.lock();
    let mut stderr = stderr.lock();

    for event in &outcome.output_events {
        match event {
            TsortOutputEvent::Stdout(range) => {
                stdout.write_all(&outcome.classic_bytes[range.clone()])?;
            }
            TsortOutputEvent::Stderr(range) => {
                stderr.write_all(&outcome.stderr_bytes[range.clone()])?;
            }
        }
    }
    if outcome.exit_code != 0 {
        set_ct_exit_code(outcome.exit_code);
    }

    Ok(())
}

pub fn tsort_native_semantic(args: impl ctcore::Args) -> CTResult<TsortSemantic> {
    let outcome = tsort_run(args)?;
    let classic_text = String::from_utf8_lossy(&outcome.classic_bytes).into_owned();
    let stderr_text = String::from_utf8_lossy(&outcome.stderr_bytes).into_owned();

    Ok(TsortSemantic {
        had_cycle: outcome.had_cycle,
        rows: outcome.rows,
        classic_text,
        classic_bytes: outcome.classic_bytes,
        stderr_text,
        exit_code: outcome.exit_code,
    })
}

fn tsort_run(args: impl ctcore::Args) -> CTResult<TsortRunOutcome> {
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    let matches = ct_app().try_get_matches_from(args)?;
    Ok(tsort_run_from_matches(&matches))
}

fn tsort_run_from_matches(matches: &clap::ArgMatches) -> TsortRunOutcome {
    let input_files: Vec<&String> = matches
        .get_many::<String>(tsort_flags::TSORT_FILE)
        .map(|v| v.collect())
        .unwrap_or_default();

    if input_files.len() > 1 {
        return tsort_error_outcome(
            format!(
                "tsort: extra operand '{}'\ntsort: Try 'tsort --help' for more information.\n",
                input_files[1]
            ),
            false,
        );
    }

    let input_file = input_files.first().map(|s| s.as_str()).unwrap_or("-");

    let input_buffer = match tsort_read_input(input_file) {
        Ok(input_buffer) => input_buffer,
        Err(stderr_text) => return tsort_error_outcome(stderr_text, false),
    };
    let mut graph = TSortGraph::new();

    let tokens = tsort_tokens(&input_buffer);

    if tokens.len() % 2 != 0 {
        return tsort_error_outcome(
            format!(
                "tsort: {}: input contains an odd number of tokens\n",
                input_file.maybe_quote()
            ),
            false,
        );
    }

    for chunk in tokens.chunks(2) {
        graph.add_edge(chunk[0].clone(), chunk[1].clone());
    }

    graph.tsort_exe(input_file)
}

pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = t!("tsort.about");
    let usage_description = t!("tsort.usage");

    // 修改：允许多个参数被解析（0个或多个），但在代码中检查数量
    // 这样可以在代码中输出 GNU 风格的错误，而不是使用 clap 的默认错误格式
    let arg = Arg::new(tsort_flags::TSORT_FILE)
        .default_value("-")
        .hide(true)
        .value_hint(clap::ValueHint::FilePath)
        .num_args(0..); // 关键修改：允许 0 到多个参数，后续手动验证

    Command::new(utility_name)
        .version(command_version)
        .about(application_info)
        .override_usage(usage_description)
        .infer_long_args(true)
        .arg(arg)
}

fn tsort_error_outcome(stderr_text: String, had_cycle: bool) -> TsortRunOutcome {
    let stderr_bytes = stderr_text.into_bytes();
    let stderr_len = stderr_bytes.len();
    TsortRunOutcome {
        had_cycle,
        rows: Vec::new(),
        classic_bytes: Vec::new(),
        stderr_bytes,
        output_events: vec![TsortOutputEvent::Stderr(0..stderr_len)],
        exit_code: 1,
    }
}

fn tsort_read_input(input_file: &str) -> Result<Vec<u8>, String> {
    let mut input: Box<dyn Read> = if input_file == "-" {
        ctcore::ct_io::stdin_reader_box()
    } else {
        let path = Path::new(input_file);
        if path.is_dir() {
            return Err(format!("tsort: {input_file}: read error: Is a directory\n"));
        }
        Box::new(
            File::open(path)
                .map_err(|err| format!("tsort: {input_file}: {}\n", strip_errno(&err)))?,
        ) as Box<dyn Read>
    };

    let mut input_buffer = Vec::new();
    input
        .read_to_end(&mut input_buffer)
        .map_err(|err| format!("tsort: {err}\n"))?;
    Ok(input_buffer)
}

fn is_tsort_delimiter(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n')
}

fn tsort_tokens(input: &[u8]) -> Vec<Vec<u8>> {
    let mut tokens = Vec::new();
    let mut index = 0;

    while index < input.len() {
        while index < input.len() && is_tsort_delimiter(input[index]) {
            index += 1;
        }
        if index == input.len() {
            break;
        }

        let start = index;
        while index < input.len() && !is_tsort_delimiter(input[index]) {
            index += 1;
        }

        let token = &input[start..index];
        let nul_index = token
            .iter()
            .position(|byte| *byte == b'\0')
            .unwrap_or(token.len());
        tokens.push(token[..nul_index].to_vec());
    }

    tokens
}

fn tsort_node_to_text(node: &[u8]) -> String {
    String::from_utf8_lossy(node).into_owned()
}

fn push_tsort_stderr(
    stderr_bytes: &mut Vec<u8>,
    output_events: &mut Vec<TsortOutputEvent>,
    message: impl AsRef<[u8]>,
) {
    let start = stderr_bytes.len();
    stderr_bytes.extend_from_slice(b"tsort: ");
    stderr_bytes.extend_from_slice(message.as_ref());
    stderr_bytes.push(b'\n');
    output_events.push(TsortOutputEvent::Stderr(start..stderr_bytes.len()));
}

fn push_tsort_stderr_node(
    stderr_bytes: &mut Vec<u8>,
    output_events: &mut Vec<TsortOutputEvent>,
    node: &[u8],
) {
    push_tsort_stderr(stderr_bytes, output_events, node);
}

use std::collections::VecDeque;

type TsortNode = Vec<u8>;

#[derive(Default)]
struct TSortGraph {
    indegrees: BTreeMap<TsortNode, usize>,
    out_edges: BTreeMap<TsortNode, Vec<TsortNode>>,
}

impl TSortGraph {
    fn new() -> Self {
        Self::default()
    }

    fn init_node(&mut self, n: TsortNode) {
        self.indegrees.entry(n.clone()).or_default();
        self.out_edges.entry(n).or_default();
    }

    fn add_edge(&mut self, from: TsortNode, to: TsortNode) {
        self.init_node(from.clone());
        self.init_node(to.clone());

        if from != to {
            *self.indegrees.get_mut(&to).unwrap() += 1;
            self.out_edges.get_mut(&from).unwrap().push(to);
        }
    }

    fn tsort_exe(&mut self, filename: &str) -> TsortRunOutcome {
        let mut found_cycle = false;
        let mut queue: VecDeque<TsortNode> = VecDeque::new();
        let mut rows = Vec::new();
        let mut classic_bytes = Vec::new();
        let mut stderr_bytes = Vec::new();
        let mut output_events = Vec::new();

        // 1. 初始化扫描 (GNU walk_tree scan_zeros)
        // BTreeMap 默认迭代顺序是 key 的升序 (Alphabetical)，这与 C 语言 walk_tree 一致
        for (node, indegree) in &self.indegrees {
            if *indegree == 0 {
                queue.push_back(node.clone());
            }
        }

        while !self.indegrees.is_empty() {
            if queue.is_empty() {
                found_cycle = true;
                if let Some(freed_node) =
                    self.detect_and_break_cycle(filename, &mut stderr_bytes, &mut output_events)
                {
                    if let Some(indegree) = self.indegrees.get(&freed_node) {
                        if *indegree == 0 {
                            queue.push_back(freed_node);
                        }
                    }
                } else {
                    break;
                }
            }

            while let Some(n) = queue.pop_front() {
                let output_start = classic_bytes.len();
                classic_bytes.extend_from_slice(&n);
                classic_bytes.push(b'\n');
                output_events.push(TsortOutputEvent::Stdout(output_start..classic_bytes.len()));
                rows.push(TsortRow {
                    row_index: rows.len() + 1,
                    node: tsort_node_to_text(&n),
                });

                if let Some(succs) = self.out_edges.remove(&n) {
                    // 模拟 C 语言 successor 链表的 LIFO 行为，必须反向遍历
                    for succ in succs.into_iter().rev() {
                        if let Some(indegree) = self.indegrees.get_mut(&succ) {
                            *indegree -= 1;
                            if *indegree == 0 {
                                queue.push_back(succ);
                            }
                        }
                    }
                }
                self.indegrees.remove(&n);
            }
        }

        TsortRunOutcome {
            had_cycle: found_cycle,
            rows,
            classic_bytes,
            stderr_bytes,
            output_events,
            exit_code: if found_cycle { 1 } else { 0 },
        }
    }

    // 完全重写：模拟 GNU tsort 的反向搜索 (Reverse Search) 算法
    fn detect_and_break_cycle(
        &mut self,
        filename: &str,
        stderr_bytes: &mut Vec<u8>,
        output_events: &mut Vec<TsortOutputEvent>,
    ) -> Option<TsortNode> {
        let candidates: Vec<TsortNode> = self.indegrees.keys().cloned().collect();

        // 对应 GNU 代码中的 static struct item *loop
        let mut cursor: Option<TsortNode> = None;

        // 对应 GNU 代码中的 qlink (记录路径: key -> value 表示 key 指向 value)
        // 注意 GNU 的 qlink 是反向链表，但在 detect_loop 中它的构建方式是：
        // 找到 k 指向 loop，则 k->qlink = loop。所以这是正向的路径记录 (Predecessor -> Successor)
        let mut qlink: HashMap<TsortNode, TsortNode> = HashMap::new();

        // 模拟 GNU 的 repeated walk_tree
        loop {
            // 每次都从头遍历所有节点 (A..Z)
            for k in &candidates {
                // 如果当前没有 cursor，找第一个有入度的节点作为起点
                if cursor.is_none() {
                    // 检查 k 是否有入度 (count > 0)
                    if self.indegrees.get(k).copied().unwrap_or(0) > 0 {
                        cursor = Some(k.clone());
                    }
                    continue;
                }

                let curr = cursor.as_ref().unwrap();

                // 核心逻辑：检查 k 是否指向 curr (即 k 是 curr 的前驱)
                // GNU: if ((*p)->suc == loop)
                let k_points_to_curr = self
                    .out_edges
                    .get(k)
                    .map(|succs| succs.contains(curr))
                    .unwrap_or(false);

                if k_points_to_curr {
                    // 检查 k 是否已经在当前路径中 (GNU: if (k->qlink))
                    if qlink.contains_key(k) {
                        // *** 发现了环 ***
                        push_tsort_stderr(
                            stderr_bytes,
                            output_events,
                            format!("{}: input contains a loop:", filename.maybe_quote()),
                        );

                        // 回溯打印环 (GNU: while (loop) ... until loop == k)
                        // 我们当前的 cursor 就是 GNU 的 loop
                        let mut loop_node = curr.clone();

                        // 1. 打印 loop_node
                        push_tsort_stderr_node(stderr_bytes, output_events, &loop_node);

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
                                if let Some(indegree) = self.indegrees.get_mut(curr) {
                                    *indegree -= 1;
                                }
                                if let Some(succs) = self.out_edges.get_mut(k) {
                                    if let Some(pos) = succs.iter().rposition(|x| x == curr) {
                                        succs.remove(pos);
                                    }
                                }
                                // 返回被释放入度的节点 (curr)
                                return Some(curr.clone());
                            }

                            push_tsort_stderr_node(stderr_bytes, output_events, &next_node);
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
            cursor.as_ref()?;
        }
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

        // 辅助函数：简化字节节点的创建
        fn s(text: &str) -> TsortNode {
            text.as_bytes().to_vec()
        }

        #[test]
        fn test_node_existence() {
            let mut graph = TSortGraph::new();
            graph.init_node(s("A"));

            // 直接检查内部 BTreeMap 是否包含键
            assert!(graph.indegrees.contains_key(&s("A")));
            assert!(!graph.indegrees.contains_key(&s("B")));

            // 检查出度表是否初始化
            assert!(graph.out_edges.contains_key(&s("A")));
        }

        #[test]
        fn test_edge_existence() {
            let mut graph = TSortGraph::new();
            graph.add_edge(s("A"), s("B"));

            // 验证节点存在
            assert!(graph.indegrees.contains_key(&s("A")));
            assert!(graph.indegrees.contains_key(&s("B")));

            // 验证边存在 (A -> B)
            // A 的出度包含 B
            assert!(graph.out_edges.get(&s("A")).unwrap().contains(&s("B")));
            // B 的入度为 1
            assert_eq!(graph.indegrees.get(&s("B")), Some(&1));

            // 验证反向边不存在
            assert!(!graph.out_edges.get(&s("B")).unwrap().contains(&s("A")));
        }

        #[test]
        fn test_init_node() {
            let mut graph = TSortGraph::new();
            graph.init_node(s("A"));

            assert!(graph.indegrees.contains_key(&s("A")));
            assert_eq!(graph.indegrees.get(&s("A")), Some(&0));
            assert!(graph.out_edges.get(&s("A")).unwrap().is_empty());
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
            let outcome = graph.tsort_exe("test");

            assert_eq!(outcome.exit_code, 0);
            assert!(outcome.stderr_bytes.is_empty());
            assert_eq!(
                outcome
                    .rows
                    .iter()
                    .map(|row| row.node.as_str())
                    .collect::<Vec<_>>(),
                vec!["A", "B", "C"]
            );
            assert!(
                graph.indegrees.is_empty(),
                "Graph should be empty after successful sort"
            );
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
            let outcome = graph.tsort_exe("test");

            assert_eq!(outcome.exit_code, 1);
            assert_eq!(
                outcome
                    .rows
                    .iter()
                    .map(|row| row.node.as_str())
                    .collect::<Vec<_>>(),
                vec!["A", "B", "C"]
            );
            assert!(
                String::from_utf8_lossy(&outcome.stderr_bytes).contains("input contains a loop")
            );

            // 即便有环，现在的逻辑也会打破它并输出所有节点，所以最终图也应该是空的
            assert!(graph.indegrees.is_empty());
        }

        #[test]
        fn test_tsort_with_multiple_start_nodes() {
            let mut graph = TSortGraph::new();
            // A -> C, B -> C
            graph.add_edge(s("A"), s("C"));
            graph.add_edge(s("B"), s("C"));

            let outcome = graph.tsort_exe("test");
            assert_eq!(outcome.exit_code, 0);
            assert!(graph.indegrees.is_empty());
        }

        #[test]
        fn test_tsort_with_no_edges() {
            let mut graph = TSortGraph::new();
            // 只有独立节点
            graph.init_node(s("A"));
            graph.init_node(s("B"));
            graph.init_node(s("C"));

            let outcome = graph.tsort_exe("test");
            assert_eq!(outcome.exit_code, 0);
            assert!(graph.indegrees.is_empty());
        }

        #[test]
        fn test_tsort_with_single_node() {
            let mut graph = TSortGraph::new();
            graph.init_node(s("A"));

            let outcome = graph.tsort_exe("test");
            assert_eq!(outcome.exit_code, 0);
            assert!(graph.indegrees.is_empty());
        }

        #[test]
        fn test_tsort_with_duplicate_edges() {
            let mut graph = TSortGraph::new();
            graph.add_edge(s("A"), s("B"));
            graph.add_edge(s("A"), s("B")); // 重复添加
            graph.add_edge(s("B"), s("C"));

            // GNU 会保留每条重复关系并分别计算入度。
            assert_eq!(graph.out_edges.get(&s("A")).unwrap().len(), 2);
            assert_eq!(graph.indegrees.get(&s("B")), Some(&2));

            let outcome = graph.tsort_exe("test");
            assert_eq!(outcome.exit_code, 0);
        }

        #[test]
        fn test_self_loop_behavior() {
            // 对应输入: A A
            // GNU tsort 逻辑：如果 from == to，则忽略边的关系，但记录节点。
            // 所以这不算“环”（Cycle），而是普通节点。
            let mut graph = TSortGraph::new();
            graph.add_edge(s("A"), s("A"));

            // 验证：节点存在，但没有自环边
            assert!(graph.indegrees.contains_key(&s("A")));
            assert_eq!(graph.indegrees.get(&s("A")), Some(&0));

            let outcome = graph.tsort_exe("test");
            assert_eq!(outcome.exit_code, 0); // 自环不视为错误
        }

        #[test]
        fn test_explicit_cycle_two_nodes() {
            // 对应输入: A B, B A
            let mut graph = TSortGraph::new();
            graph.add_edge(s("A"), s("B"));
            graph.add_edge(s("B"), s("A"));

            let outcome = graph.tsort_exe("test");
            assert_eq!(outcome.exit_code, 1); // 存在真正的环
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
            ctcore::ct_error::set_ct_exit_code(0);
            let file_name = "test_tsort_main_execution_default_nul_file";

            let args = [ctcore::ct_util_name(), file_name];
            let result = tsort_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
            assert_eq!(ctcore::ct_error::get_ct_exit_code(), 1);
            ctcore::ct_error::set_ct_exit_code(0);
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
            ctcore::ct_error::set_ct_exit_code(0);
            let dir = tempdir().unwrap();
            let file_path = dir
                .path()
                .join("test_tsort_main_execution_default_file_data");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(tmp_file, "a b c\nc d").unwrap();

            let file_name = file_path.to_str().unwrap();

            let args = [ctcore::ct_util_name(), file_name];
            let result = tsort_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
            assert_eq!(ctcore::ct_error::get_ct_exit_code(), 1);
            ctcore::ct_error::set_ct_exit_code(0);
        }

        #[test]
        fn test_tsort_main_execution_default_file_data_err() {
            ctcore::ct_error::set_ct_exit_code(0);
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("sort_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(tmp_file, "a b\nb a").unwrap();
            let file_name = file_path.to_str().unwrap();

            let args = [ctcore::ct_util_name(), file_name];
            let result = tsort_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
            assert_eq!(ctcore::ct_error::get_ct_exit_code(), 1);
            ctcore::ct_error::set_ct_exit_code(0);
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

        #[test]
        fn test_tsort_native_semantic_collects_rows_and_metadata() {
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("graph.txt");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(tmp_file, "a b\nb c\nc d").unwrap();

            let semantic = tsort_native_semantic(
                vec![
                    OsString::from("tsort"),
                    file_path.as_os_str().to_os_string(),
                ]
                .into_iter(),
            )
            .unwrap();

            assert!(!semantic.had_cycle);
            assert_eq!(semantic.stderr_text, "");
            assert_eq!(semantic.exit_code, 0);
            assert_eq!(semantic.classic_text, "a\nb\nc\nd\n");
            assert_eq!(semantic.classic_bytes, b"a\nb\nc\nd\n");
            assert_eq!(
                semantic.rows,
                vec![
                    TsortRow {
                        row_index: 1,
                        node: "a".into(),
                    },
                    TsortRow {
                        row_index: 2,
                        node: "b".into(),
                    },
                    TsortRow {
                        row_index: 3,
                        node: "c".into(),
                    },
                    TsortRow {
                        row_index: 4,
                        node: "d".into(),
                    },
                ]
            );
        }

        #[test]
        fn test_tsort_native_semantic_preserves_cycle_output() {
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("cycle.txt");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(tmp_file, "a b\nb a").unwrap();

            let semantic = tsort_native_semantic(
                vec![
                    OsString::from("tsort"),
                    file_path.as_os_str().to_os_string(),
                ]
                .into_iter(),
            )
            .unwrap();

            assert!(semantic.had_cycle);
            assert_eq!(semantic.exit_code, 1);
            assert_eq!(semantic.classic_text, "a\nb\n");
            assert_eq!(semantic.classic_bytes, b"a\nb\n");
            assert_eq!(
                semantic.stderr_text,
                format!(
                    "tsort: {}: input contains a loop:\ntsort: a\ntsort: b\n",
                    file_path.display()
                )
            );
        }

        #[test]
        fn test_tsort_native_semantic_cr_is_not_a_token_delimiter() {
            let semantic = ctcore::ct_io::with_injected_stdin(b"\r\n".to_vec(), || {
                tsort_native_semantic(vec![OsString::from("tsort")].into_iter())
            })
            .unwrap();

            assert_eq!(semantic.exit_code, 1);
            assert_eq!(
                semantic.stderr_text,
                "tsort: -: input contains an odd number of tokens\n"
            );
            assert!(semantic.classic_bytes.is_empty());
        }

        #[test]
        fn test_tsort_native_semantic_accepts_non_utf8_and_nul_tokens() {
            let semantic = ctcore::ct_io::with_injected_stdin(b"a\0b c\n\xff d\n".to_vec(), || {
                tsort_native_semantic(vec![OsString::from("tsort")].into_iter())
            })
            .unwrap();

            assert_eq!(semantic.exit_code, 0);
            assert_eq!(semantic.classic_bytes, b"a\n\xff\nc\nd\n");
            assert_eq!(
                semantic.classic_text,
                String::from_utf8_lossy(b"a\n\xff\nc\nd\n").into_owned()
            );
            assert_eq!(
                semantic
                    .rows
                    .iter()
                    .map(|row| row.node.as_str())
                    .collect::<Vec<_>>(),
                vec!["a", "\u{fffd}", "c", "d"]
            );
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
