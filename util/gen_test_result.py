"""
此脚本用于从 GNU 命令测试的日志文件中提取测试结果，并生成一个汇总的 JSON 文件
每个命令的测试结果存储在 ../gnu/tests/<command>/ 目录下的 .log 文件中
生成的 JSON 文件路径为 ./aggregated-result.json
"""

import json
from pathlib import Path

# GNU 命令列表
commands = [
    "basenc",
    "cat",
    "chcon",
    "chgrp",
    "chmod",
    "chown",
    "chroot",
    "cksum",
    "cp",
    "csplit",
    "cut",
    "date",
    "dd",
    "df",
    "du",
    "env",
    "expr",
    "factor",
    "fmt",
    "fold",
    "groups",
    "head",
    "id",
    "install",
    "join",
    "ln",
    "ls",
    "misc",
    "mkdir",
    "mv",
    "nice",
    "nproc",
    "numfmt",
    "od",
    "pr",
    "printf",
    "ptx",
    "pwd",
    "readlink",
    "rm",
    "rmdir",
    "runcon",
    "seq",
    "shred",
    "shuf",
    "sort",
    "split",
    "stat",
    "stty",
    "tac",
    "tail",
    "test",
    "timeout",
    "touch",
    "tr",
    "truncate",
    "tty",
    "uniq",
    "wc",
]

def extract_result_from_log(log_path: Path) -> str | None:
    """
    从 .log 文件中提取测试结果
    """
    try:
        with log_path.open("r", encoding="utf-8", errors="ignore") as f:
            lines = f.readlines()
    except OSError:
        return None

    # 从后向前找
    for line in reversed(lines):
        line = line.strip()
        if not line:
            continue
        if line.startswith(("PASS", "FAIL", "SKIP", "ERROR", "Failed")):
            return line.split()[0]

    return "Unknown"

def main():
    # 当前脚本所在目录
    script_dir = Path(__file__).resolve().parent

    # ../gnu/tests
    tests_dir = script_dir.parent.parent / "gnu" / "tests"

    if not tests_dir.is_dir():
        print(f"Tests directory not found: {tests_dir}")
        return

    result = {}

    for cmd in commands:
        cmd_dir = tests_dir / cmd
        if not cmd_dir.is_dir():
            # 命令目录不存在，直接跳过
            print(f"Tests results for command {cmd} not found. Skipping.")
            continue

        cmd_results = {}

        for log_file in cmd_dir.glob("*.log"):
            status = extract_result_from_log(log_file)
            if status is None:
                continue
            if status == "Failed":
                status = "FAIL"
            cmd_results[log_file.name] = status

        if cmd_results:
            result[cmd] = cmd_results

    # 输出 JSON 文件
    output_path = script_dir / "aggregated-result.json"
    with output_path.open("w", encoding="utf-8") as f:
        json.dump(result, f, indent=2, ensure_ascii=False)

    print(f"Generated successfully: {output_path}")

if __name__ == "__main__":
    main()