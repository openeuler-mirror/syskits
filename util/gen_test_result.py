"""
gen_test_result.py
Version: 2026.03.26

用途：
1. 从 GNU 命令测试日志中提取结果，生成 `aggregated-result.json`
2. 根据结果数据生成可视化页面 `test_coverage.html`
3. 支持列出测试组件、测试用例和已有测试结果

主要能力：
- 仅基于现有日志生成结果
- 先调用 `run-gnu-test.sh` 跑全量测试，再生成结果
- 先调用 `run-gnu-test.sh` 跑指定测试脚本，再生成结果
- 列出 `gnu/tests` 下的测试组件和用例
- 从 `util/aggregated-result.json` 列出已有测试结果

常用示例：
- `python3 gen_test_result.py`
- `python3 gen_test_result.py --run`
- `python3 gen_test_result.py --run tests/tail/tail-c.sh`
- `python3 gen_test_result.py --list-tests`
- `python3 gen_test_result.py --list-tests tail`
- `python3 gen_test_result.py --list-results`
- `python3 gen_test_result.py --list-results tail`
"""

import argparse
import json
import os
import subprocess
from datetime import datetime
from pathlib import Path

TEST_SCRIPT_SUFFIXES = {".sh", ".pl"}
SCRIPT_VERSION = "2026.03.26"
RESULT_FILTER_STATUSES = ("PASS", "FAIL", "SKIP", "ERROR")


def parse_args() -> argparse.Namespace:
    """
    解析命令行参数。
    """
    parser = argparse.ArgumentParser(
        description="聚合 GNU 测试日志并生成 HTML，可选先运行测试。"
    )
    parser.add_argument(
        "--run",
        action="store_true",
        help="先调用 util/run-gnu-test.sh 运行测试，再生成结果。",
    )
    parser.add_argument(
        "--debug",
        action="store_true",
        help="调用 run-gnu-test.sh 时附带 DEBUG=1 环境变量。",
    )
    parser.add_argument(
        "--list-tests",
        action="store_true",
        help="列出 gnu/tests 下的测试组件及其用例绝对路径；可在后面追加组件名进行过滤。",
    )
    parser.add_argument(
        "--list-results",
        action="store_true",
        help="从 util/aggregated-result.json 读取测试结果；可在后面追加组件名进行过滤。",
    )
    parser.add_argument(
        "--pass",
        dest="status_pass",
        action="store_true",
        help="在 --list-results 模式下仅显示 PASS 结果。",
    )
    parser.add_argument(
        "--fail",
        dest="status_fail",
        action="store_true",
        help="在 --list-results 模式下仅显示 FAIL 结果。",
    )
    parser.add_argument(
        "--skip",
        dest="status_skip",
        action="store_true",
        help="在 --list-results 模式下仅显示 SKIP 结果。",
    )
    parser.add_argument(
        "--error",
        dest="status_error",
        action="store_true",
        help="在 --list-results 模式下仅显示 ERROR 结果。",
    )
    parser.add_argument(
        "tests",
        nargs="*",
        help="可选的 GNU 测试路径，例如 tests/tail/tail-c.sh；在 --list-tests / --list-results 模式下也可传组件名，例如 tail。",
    )
    return parser.parse_args()


def extract_result_from_log(log_path: Path) -> str | None:
    """
    从 .log 文件末尾提取测试状态。
    """
    try:
        with log_path.open("r", encoding="utf-8", errors="ignore") as file:
            lines = file.readlines()
    except OSError:
        return None

    for line in reversed(lines):
        line = line.strip()
        if not line:
            continue
        if line.startswith(("PASS", "FAIL", "SKIP", "ERROR", "Failed")):
            status = line.split()[0]
            return "FAIL" if status == "Failed" else status
        if "exit status:" in line:
            return "PASS" if "exit status: 0" in line else "FAIL"

    return "UNKNOWN"


def normalize_test_path(test: str, gnu_dir: Path) -> Path:
    """
    将传入的测试路径规范化为相对 gnu 根目录的路径。
    """
    candidate = Path(test)
    if candidate.is_absolute():
        try:
            relative_path = candidate.relative_to(gnu_dir)
        except ValueError as exc:
            raise ValueError(f"Test path is outside GNU directory: {candidate}") from exc
    else:
        relative_path = candidate

    if not relative_path.parts:
        raise ValueError("Test path cannot be empty.")

    if relative_path.parts[0] != "tests":
        relative_path = Path("tests") / relative_path

    return relative_path


def test_to_log_path(normalized_test_path: Path) -> Path:
    """
    根据测试脚本路径推导对应的 .log 路径。
    """
    if normalized_test_path.suffix == ".log":
        return normalized_test_path
    if normalized_test_path.suffix in TEST_SCRIPT_SUFFIXES:
        return normalized_test_path.with_suffix(".log")
    return normalized_test_path.with_suffix(".log")


def discover_all_log_files(tests_dir: Path) -> list[Path]:
    """
    发现 tests 目录下所有测试日志。
    """
    return sorted(
        log_file
        for log_file in tests_dir.rglob("*.log")
        if log_file.is_file() and log_file.parent != tests_dir
    )


def discover_test_cases(tests_dir: Path) -> dict[str, list[Path]]:
    """
    发现 tests 目录下的测试组件及其用例。
    """
    test_components: dict[str, list[Path]] = {}

    for component_dir in sorted(
        path for path in tests_dir.iterdir() if path.is_dir()
    ):
        cases = sorted(
            test_case.resolve()
            for suffix in TEST_SCRIPT_SUFFIXES
            for test_case in component_dir.glob(f"*{suffix}")
            if test_case.is_file()
        )
        if cases:
            test_components[str(component_dir.resolve())] = cases

    return test_components


def print_test_cases(test_components: dict[str, list[Path]]) -> None:
    """
    输出测试组件及其用例列表。
    """
    total_components = len(test_components)
    total_cases = sum(len(cases) for cases in test_components.values())

    print(f"Found {total_components} test components and {total_cases} test cases.")
    for component_path, cases in test_components.items():
        component_name = Path(component_path).name
        print()
        print(f"Component: {component_name}")
        print(f"Path: {component_path}")
        for case_path in cases:
            print(f"  - {case_path}")


def load_aggregated_results(results_path: Path) -> dict[str, dict[str, object]]:
    """
    读取已有的 aggregated-result.json。
    """
    if not results_path.is_file():
        raise FileNotFoundError(f"Results file not found: {results_path}")

    with results_path.open("r", encoding="utf-8") as file:
        data = json.load(file)

    if not isinstance(data, dict):
        raise ValueError(f"Invalid results file format: {results_path}")

    return data


def normalize_result_entry(raw_entry: object) -> dict[str, str]:
    """
    兼容新旧结果格式，统一成带路径字段的结构。
    """
    if isinstance(raw_entry, str):
        return {
            "status": raw_entry,
            "test_file_path": "",
            "log_file_path": "",
        }

    if isinstance(raw_entry, dict):
        return {
            "status": str(raw_entry.get("status", "UNKNOWN")),
            "test_file_path": str(raw_entry.get("test_file_path", "")),
            "log_file_path": str(raw_entry.get("log_file_path", "")),
        }

    return {
        "status": "UNKNOWN",
        "test_file_path": "",
        "log_file_path": "",
    }


def build_result_components(
    raw_results: dict[str, dict[str, object]],
) -> dict[str, dict[str, object]]:
    """
    将结果 JSON 组织成便于过滤和打印的组件结构。
    """
    result_components: dict[str, dict[str, object]] = {}

    for component_name, raw_cases in sorted(raw_results.items()):
        if not isinstance(raw_cases, dict):
            continue

        normalized_cases: dict[str, dict[str, str]] = {}
        component_path = ""

        for case_name, raw_entry in sorted(raw_cases.items()):
            normalized_entry = normalize_result_entry(raw_entry)
            normalized_cases[case_name] = normalized_entry

            if not component_path:
                reference_path = (
                    normalized_entry["test_file_path"] or normalized_entry["log_file_path"]
                )
                if reference_path:
                    component_path = str(Path(reference_path).parent)

        result_components[component_name] = {
            "component_path": component_path,
            "cases": normalized_cases,
        }

    return result_components


def select_result_components(
    result_components: dict[str, dict[str, object]],
    component_filters: list[str],
) -> dict[str, dict[str, object]]:
    """
    根据组件过滤条件筛选测试结果组件。
    支持：
    - 组件名: tail
    - 相对路径: tests/tail
    - 绝对路径: /abs/path/to/gnu/tests/tail
    """
    if not component_filters:
        return result_components

    selected_components: dict[str, dict[str, object]] = {}
    unmatched_filters: list[str] = []

    for component_filter in component_filters:
        normalized_filter = component_filter.replace("\\", "/").rstrip("/")
        matched = False

        for component_name, component_data in result_components.items():
            component_path = str(component_data.get("component_path", ""))
            normalized_component_path = component_path.replace("\\", "/").rstrip("/")

            if (
                normalized_filter == component_name
                or (
                    normalized_component_path
                    and (
                        normalized_filter == normalized_component_path
                        or normalized_component_path.endswith(f"/{normalized_filter}")
                    )
                )
            ):
                selected_components[component_name] = component_data
                matched = True

        if not matched:
            unmatched_filters.append(component_filter)

    if unmatched_filters:
        missing = ", ".join(unmatched_filters)
        raise ValueError(f"Result component not found: {missing}")

    return dict(sorted(selected_components.items()))


def print_test_results(result_components: dict[str, dict[str, object]]) -> None:
    """
    输出测试组件及其结果列表。
    """
    total_components = len(result_components)
    total_cases = sum(
        len(component_data.get("cases", {}))
        for component_data in result_components.values()
    )

    print(f"Found {total_components} result components and {total_cases} test results.")
    for component_name, component_data in result_components.items():
        component_path = str(component_data.get("component_path", ""))
        cases = component_data.get("cases", {})

        print()
        print(f"Component: {component_name}")
        if component_path:
            print(f"Path: {component_path}")

        if not isinstance(cases, dict):
            continue

        for case_name, raw_entry in cases.items():
            entry = normalize_result_entry(raw_entry)
            print(f"  - {case_name} [{entry['status']}]")
            if entry["test_file_path"]:
                print(f"    Test File: {entry['test_file_path']}")
            if entry["log_file_path"]:
                print(f"    Log File: {entry['log_file_path']}")


def get_result_status_filters(args: argparse.Namespace) -> set[str]:
    """
    从命令行参数中提取结果状态过滤条件。
    """
    status_filters: set[str] = set()
    if args.status_pass:
        status_filters.add("PASS")
    if args.status_fail:
        status_filters.add("FAIL")
    if args.status_skip:
        status_filters.add("SKIP")
    if args.status_error:
        status_filters.add("ERROR")
    return status_filters


def filter_result_components_by_status(
    result_components: dict[str, dict[str, object]],
    status_filters: set[str],
) -> dict[str, dict[str, object]]:
    """
    按状态过滤结果组件，仅保留命中的测试项和组件。
    """
    if not status_filters:
        return result_components

    filtered_components: dict[str, dict[str, object]] = {}

    for component_name, component_data in result_components.items():
        cases = component_data.get("cases", {})
        if not isinstance(cases, dict):
            continue

        filtered_cases = {
            case_name: raw_entry
            for case_name, raw_entry in cases.items()
            if normalize_result_entry(raw_entry)["status"] in status_filters
        }

        if filtered_cases:
            filtered_components[component_name] = {
                "component_path": component_data.get("component_path", ""),
                "cases": filtered_cases,
            }

    return filtered_components


def select_test_components(
    test_components: dict[str, list[Path]],
    component_filters: list[str],
) -> dict[str, list[Path]]:
    """
    根据组件过滤条件筛选测试组件。
    支持：
    - 组件名: tail
    - 相对路径: tests/tail
    - 绝对路径: /abs/path/to/gnu/tests/tail
    """
    if not component_filters:
        return test_components

    selected_components: dict[str, list[Path]] = {}
    unmatched_filters: list[str] = []

    for component_filter in component_filters:
        normalized_filter = component_filter.replace("\\", "/").rstrip("/")
        matched = False

        for component_path, cases in test_components.items():
            normalized_component_path = component_path.replace("\\", "/").rstrip("/")
            component_name = Path(component_path).name

            if (
                normalized_filter == component_name
                or normalized_filter == normalized_component_path
                or normalized_component_path.endswith(f"/{normalized_filter}")
            ):
                selected_components[component_path] = cases
                matched = True

        if not matched:
            unmatched_filters.append(component_filter)

    if unmatched_filters:
        missing = ", ".join(unmatched_filters)
        raise ValueError(f"Test component not found: {missing}")

    return dict(sorted(selected_components.items()))


def select_log_files(gnu_dir: Path, selected_tests: list[str]) -> list[Path]:
    """
    根据指定测试路径收集需要展示的日志文件。
    """
    selected_logs: list[Path] = []
    seen: set[Path] = set()

    for test in selected_tests:
        normalized_test_path = normalize_test_path(test, gnu_dir)
        log_path = gnu_dir / test_to_log_path(normalized_test_path)
        if log_path not in seen:
            selected_logs.append(log_path)
            seen.add(log_path)

    return selected_logs


def resolve_test_file_path(log_file: Path) -> Path | None:
    """
    根据日志路径推导对应的测试脚本绝对路径。
    """
    for suffix in TEST_SCRIPT_SUFFIXES:
        candidate = log_file.with_suffix(suffix)
        if candidate.is_file():
            return candidate.resolve()
    return None


def collect_results(
    log_files: list[Path],
    tests_dir: Path,
) -> dict[str, dict[str, dict[str, str]]]:
    """
    根据日志文件列表收集测试结果。
    """
    results: dict[str, dict[str, dict[str, str]]] = {}

    for log_file in sorted(log_files):
        try:
            relative_path = log_file.relative_to(tests_dir)
        except ValueError:
            continue

        if relative_path.parent == Path("."):
            continue

        group_name = relative_path.parent.as_posix()
        status = extract_result_from_log(log_file) if log_file.is_file() else "UNKNOWN"
        test_file_path = resolve_test_file_path(log_file)
        results.setdefault(group_name, {})[relative_path.name] = {
            "status": status or "UNKNOWN",
            "test_file_path": str(test_file_path) if test_file_path else "",
            "log_file_path": str(log_file.resolve()),
        }

    return results


def build_html(
    data: dict[str, dict[str, dict[str, str]]],
    scope_label: str,
    generated_at: str,
) -> str:
    """
    将测试结果渲染为单文件 HTML。
    """
    return f"""<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<title>GNU Tests Results</title>
<style>
:root {{
    --PASS: #44AF69;
    --ERROR: #F8333C;
    --FAIL: #F8333C;
    --SKIP: #d3c994;
    --UNKNOWN: #ff9f1c;
}}
.PASS {{ color: var(--PASS); }}
.ERROR {{ color: var(--ERROR); }}
.FAIL {{ color: var(--FAIL); }}
.SKIP {{ color: var(--SKIP); }}
.UNKNOWN {{ color: var(--UNKNOWN); }}
.testSummary {{
    display: inline-flex;
    align-items: center;
    justify-content: space-between;
    width: 90%;
}}
.progress {{
    width: 80%;
    display: flex;
    justify-content: right;
    align-items: center;
}}
.progress-bar {{
    height: 10px;
    width: calc(100% - 15ch);
    border-radius: 5px;
}}
.result {{
    font-weight: bold;
    width: 7ch;
    display: inline-block;
}}
.result-line {{ margin: 8px; }}
.result-main {{ margin-bottom: 4px; }}
.path-line {{
    margin-left: 7.5ch;
    color: #555;
    font-family: monospace;
    font-size: 13px;
    line-height: 1.5;
}}
.path-label {{
    color: #777;
    margin-right: 8px;
}}
.path-link {{
    color: #0b5fff;
    text-decoration: none;
}}
.path-link:hover {{
    text-decoration: underline;
}}
.counts {{ margin-right: 10px; }}
body > p {{ color: #555; }}
body {{ font-family: Arial, sans-serif; margin: 20px; }}
</style>
</head>
<body>
<h1>GNU Tests Results</h1>
<p>{scope_label}</p>
<p>Generated at: {generated_at}</p>
<div id="test-cov"></div>

<script>
const data = {json.dumps(data, indent=2, ensure_ascii=False)};

function progressBar(totals) {{
    const bar = document.createElement("div");
    bar.className = "progress-bar";

    let totalTests = 0;
    for (const value of Object.values(totals)) {{
        totalTests += value;
    }}

    if (totalTests === 0) {{
        bar.style.background = "var(--SKIP)";
    }} else {{
        const passPercentage = Math.round(100 * totals.PASS / totalTests);
        const skipPercentage = passPercentage + Math.round(100 * totals.SKIP / totalTests);
        const unknownPercentage = skipPercentage + Math.round(100 * totals.UNKNOWN / totalTests);

        bar.style = `background: linear-gradient(
            to right,
            var(--PASS) ${{passPercentage}}%,
            var(--SKIP) ${{passPercentage}}%,
            var(--SKIP) ${{skipPercentage}}%,
            var(--UNKNOWN) ${{skipPercentage}}%,
            var(--UNKNOWN) ${{unknownPercentage}}%,
            var(--FAIL) ${{unknownPercentage}}%
        )`;
    }}

    const progress = document.createElement("div");
    progress.className = "progress";
    progress.innerHTML = `
        <span class="counts">
        <span class="PASS">${{totals.PASS}}</span>
        /
        <span class="SKIP">${{totals.SKIP}}</span>
        /
        <span class="UNKNOWN">${{totals.UNKNOWN}}</span>
        /
        <span class="FAIL">${{totals.FAIL + totals.ERROR}}</span>
        </span>
    `;
    progress.appendChild(bar);
    return progress;
}}

function isLeafResult(content) {{
    return (
        content !== null &&
        typeof content === "object" &&
        Object.prototype.hasOwnProperty.call(content, "status")
    );
}}

function createPathLine(label, filePath) {{
    if (!filePath) {{
        return null;
    }}

    const line = document.createElement("div");
    line.className = "path-line";

    const labelSpan = document.createElement("span");
    labelSpan.className = "path-label";
    labelSpan.textContent = `${{label}}:`;
    line.appendChild(labelSpan);

    const link = document.createElement("a");
    link.className = "path-link";
    link.href = `file://${{encodeURI(filePath)}}`;
    link.target = "_blank";
    link.rel = "noopener noreferrer";
    link.textContent = filePath;
    line.appendChild(link);

    return line;
}}

function parseResult(parent, obj) {{
    const totals = {{
        PASS: 0,
        SKIP: 0,
        FAIL: 0,
        ERROR: 0,
        UNKNOWN: 0,
    }};

    for (const [category, content] of Object.entries(obj)) {{
        if (typeof content === "string" || isLeafResult(content)) {{
            const resultItem = document.createElement("div");
            const leaf = typeof content === "string"
                ? {{ status: content, test_file_path: "", log_file_path: "" }}
                : content;
            const status = Object.prototype.hasOwnProperty.call(totals, leaf.status)
                ? leaf.status
                : "UNKNOWN";

            resultItem.className = "result-line";
            totals[status] += 1;

            const mainLine = document.createElement("div");
            mainLine.className = "result-main";
            mainLine.innerHTML = `<span class="result" style="color: var(--${{status}})">${{status}}</span> ${{category}}`;
            resultItem.appendChild(mainLine);

            const testFileLine = createPathLine("Test File", leaf.test_file_path);
            if (testFileLine) {{
                resultItem.appendChild(testFileLine);
            }}

            const logFileLine = createPathLine("Log File", leaf.log_file_path);
            if (logFileLine) {{
                resultItem.appendChild(logFileLine);
            }}

            parent.appendChild(resultItem);
        }} else {{
            const categoryName = document.createElement("code");
            categoryName.textContent = category;

            const details = document.createElement("details");
            const subtotals = parseResult(details, content);
            for (const [subtotal, count] of Object.entries(subtotals)) {{
                totals[subtotal] += count;
            }}

            const summaryDiv = document.createElement("div");
            summaryDiv.className = "testSummary";
            summaryDiv.appendChild(categoryName);
            summaryDiv.appendChild(progressBar(subtotals));

            const summary = document.createElement("summary");
            summary.appendChild(summaryDiv);

            details.appendChild(summary);
            parent.appendChild(details);
        }}
    }}

    return totals;
}}

window.onload = () => {{
    const parent = document.getElementById("test-cov");
    parseResult(parent, data);
}};
</script>
</body>
</html>
"""


def run_tests(
    run_script_path: Path,
    workdir: Path,
    selected_tests: list[str],
    debug: bool,
) -> None:
    """
    调用 run-gnu-test.sh 执行测试。
    """
    command = ["bash", str(run_script_path), *selected_tests]
    env = os.environ.copy()
    if debug:
        env["DEBUG"] = "1"

    if selected_tests:
        print(f"Running specific tests: {' '.join(selected_tests)}")
    else:
        print("Running full GNU test suite.")

    subprocess.run(command, cwd=workdir, env=env, check=True)


def build_scope_label(ran_tests: bool, selected_tests: list[str]) -> str:
    """
    生成页面顶部的结果范围描述。
    """
    if selected_tests:
        return f"Scope: {', '.join(selected_tests)}"
    if ran_tests:
        return "Scope: full GNU test suite"
    return "Scope: existing GNU test logs"


def validate_cli_args(
    args: argparse.Namespace,
    status_filters: set[str],
) -> str | None:
    """
    校验命令行参数组合，返回错误信息或 None。
    """
    if args.list_tests and args.list_results:
        return "--list-tests and --list-results cannot be used together."

    if status_filters and not args.list_results:
        filters_text = ", ".join(sorted(status_filters))
        return f"Status filters require --list-results: {filters_text}"

    return None


def handle_list_results_mode(
    results_path: Path,
    component_filters: list[str],
    status_filters: set[str],
) -> int:
    """
    处理 --list-results 模式。
    """
    try:
        result_components = select_result_components(
            build_result_components(load_aggregated_results(results_path)),
            component_filters,
        )
    except (FileNotFoundError, ValueError) as exc:
        print(exc)
        return 1

    result_components = filter_result_components_by_status(
        result_components,
        status_filters,
    )
    print_test_results(result_components)
    return 0


def handle_list_tests_mode(tests_dir: Path, component_filters: list[str]) -> int:
    """
    处理 --list-tests 模式。
    """
    if not tests_dir.is_dir():
        print(f"Tests directory not found: {tests_dir}")
        return 1

    try:
        test_components = select_test_components(
            discover_test_cases(tests_dir),
            component_filters,
        )
    except ValueError as exc:
        print(exc)
        return 1

    print_test_cases(test_components)
    return 0


def normalize_requested_tests(requested_tests: list[str], gnu_dir: Path) -> list[str]:
    """
    规范化命令行传入的测试路径。
    """
    return [
        normalize_test_path(test, gnu_dir).as_posix()
        for test in requested_tests
    ]


def write_result_outputs(
    script_dir: Path,
    results: dict[str, dict[str, dict[str, str]]],
    ran_tests: bool,
    normalized_tests: list[str],
) -> None:
    """
    将聚合结果写入 JSON 和 HTML 文件。
    """
    json_output_path = script_dir / "aggregated-result.json"
    with json_output_path.open("w", encoding="utf-8") as file:
        json.dump(results, file, indent=2, ensure_ascii=False)

    html_output_path = script_dir / "test_coverage.html"
    generated_at = datetime.now().astimezone().strftime("%Y-%m-%d %H:%M:%S %Z")
    scope_label = build_scope_label(ran_tests, normalized_tests)
    html_output_path.write_text(
        build_html(results, scope_label, generated_at),
        encoding="utf-8",
    )

    print(f"Generated successfully: {json_output_path}")
    print(f"Generated successfully: {html_output_path}")


def handle_generate_mode(
    args: argparse.Namespace,
    script_dir: Path,
    gnu_dir: Path,
    tests_dir: Path,
    run_script_path: Path,
) -> int:
    """
    处理默认的结果生成模式。
    """
    if not run_script_path.is_file():
        print(f"Run script not found: {run_script_path}")
        return 1

    if not tests_dir.is_dir():
        print(f"Tests directory not found: {tests_dir}")
        return 1

    try:
        normalized_tests = normalize_requested_tests(args.tests, gnu_dir)
    except ValueError as exc:
        print(exc)
        return 1

    if args.run:
        run_tests(run_script_path, script_dir, normalized_tests, args.debug)

    log_files = (
        select_log_files(gnu_dir, normalized_tests)
        if normalized_tests
        else discover_all_log_files(tests_dir)
    )
    results = collect_results(log_files, tests_dir)
    write_result_outputs(script_dir, results, args.run, normalized_tests)
    return 0


def main() -> int:
    args = parse_args()
    script_dir = Path(__file__).resolve().parent
    repo_dir = script_dir.parent
    gnu_dir = repo_dir.parent / "gnu"
    tests_dir = gnu_dir / "tests"
    run_script_path = script_dir / "run-gnu-test.sh"
    results_path = script_dir / "aggregated-result.json"
    status_filters = get_result_status_filters(args)

    error_message = validate_cli_args(args, status_filters)
    if error_message:
        print(error_message)
        return 1

    if args.list_results:
        return handle_list_results_mode(
            results_path,
            args.tests,
            status_filters,
        )

    if args.list_tests:
        return handle_list_tests_mode(tests_dir, args.tests)

    return handle_generate_mode(
        args,
        script_dir,
        gnu_dir,
        tests_dir,
        run_script_path,
    )


if __name__ == "__main__":
    raise SystemExit(main())
