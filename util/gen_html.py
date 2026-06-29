"""
获取之前生成的测试结果 JSON 文件，生成一个静态 HTML 页面用于展示测试结果
"""

import json

# 读取之前生成的测试结果 JSON
with open("aggregated-result.json", "r", encoding="utf-8") as f:
    data = json.load(f)

html_content = f"""
<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<title>GNU Tests Results</title>
<style>
/* === test_coverage.css === */
:root {{
    --PASS: #44AF69;
    --ERROR: #F8333C;
    --FAIL: #F8333C;
    --SKIP: #d3c994;
}}
.PASS {{ color: var(--PASS); }}
.ERROR {{ color: var(--ERROR); }}
.FAIL {{ color: var(--FAIL); }}
.SKIP {{ color: var(--SKIP); }}
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
.counts {{ margin-right: 10px; }}
body {{ font-family: Arial, sans-serif; margin: 20px; }}
</style>
</head>
<body>
<h1>GNU Tests Results</h1>
<div id="test-cov"></div>

<script>
/* === 内嵌 JSON 数据 === */
const data = {json.dumps(data, indent=2)};

/* === test_coverage.js === */
function progressBar(totals) {{
    const bar = document.createElement("div");
    bar.className = "progress-bar";
    let totalTests = 0;
    for (const [key, value] of Object.entries(totals)) {{
        totalTests += value;
    }}
    const passPercentage = Math.round(100 * totals["PASS"] / totalTests);
    const skipPercentage = passPercentage + Math.round(100 * totals["SKIP"] / totalTests);

    bar.style = `background: linear-gradient(
        to right,
        var(--PASS) ${{passPercentage}}%`
        + ( passPercentage === 100 ? ", var(--PASS)" :
        `, var(--SKIP) ${{passPercentage}}%,
        var(--SKIP) ${{skipPercentage}}%`
        )
        + (skipPercentage === 100 ? ")" : ", var(--FAIL) 0)");

    const progress = document.createElement("div");
    progress.className = "progress"
    progress.innerHTML = `
        <span class="counts">
        <span class="PASS">${{totals["PASS"]}}</span>
        /
        <span class="SKIP">${{totals["SKIP"]}}</span>
        /
        <span class="FAIL">${{totals["FAIL"] + totals["ERROR"]}}</span>
        </span>
    `;
    progress.appendChild(bar);
    return progress
}}

function parse_result(parent, obj) {{
    const totals = {{
        PASS: 0,
        SKIP: 0,
        FAIL: 0,
        ERROR: 0,
    }};
    for (const [category, content] of Object.entries(obj)) {{
        if (typeof content === "string") {{
            const p = document.createElement("p");
            p.className = "result-line";
            totals[content]++;
            p.innerHTML = `<span class="result" style="color: var(--${{content}})">${{content}}</span> ${{category}}`;
            parent.appendChild(p);
        }} else {{
            const categoryName = document.createElement("code");
            categoryName.innerHTML = category;
            categoryName.className = "hljs";

            const details = document.createElement("details");
            const subtotals = parse_result(details, content);
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

/* 渲染页面 */
window.onload = () => {{
    let parent = document.getElementById("test-cov");
    parse_result(parent, data);
}};
</script>
</body>
</html>
"""

# 写入静态 HTML 文件
with open("test_coverage.html", "w", encoding="utf-8") as f:
    f.write(html_content)

print("Generated successfully: test_coverage.html")