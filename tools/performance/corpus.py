#!/usr/bin/env python3
"""Generate the versioned editor acceptance vault; never overwrite an existing path."""
import argparse
import hashlib
import json
from pathlib import Path

VERSION = 1


def sources():
    paragraph = "中文输入、English text、组合字符 e\u0301 和 emoji 👩‍💻，用于检查光标与选区。"
    ordinary = "# 日常笔记\n\n" + "\n\n".join(
        f"## 小节 {i}\n\n{paragraph}\n\n- 待办事项\n- [ ] 检查保存\n\n**重点**与[内部链接](ordinary.md)。"
        for i in range(20)
    ) + "\n"
    yield "ordinary.md", ordinary
    yield "long-paragraph.md", "# 长段落\n\n" + paragraph * 10_000 + "\n"
    yield "large-document.md", "# 大文档\n\n" + "\n\n".join(
        f"## 段落 {i}\n\n{paragraph * 8}\n\n```rust\nlet value_{i} = {i};\n```"
        for i in range(2_000)
    ) + "\n"
    yield "formula-table.md", "# 公式与表格\n\n" + "\n\n".join(
        f"## 计算 {i}\n\n$\\frac{{x_{{{i}}}^2}}{{1+x_{{{i}}}}}$\n\n"
        f"| 项目 | 数值 |\n| --- | --- |\n| 中文 {i} | {i * 7} |\n| 合计 | **{i * 11}** |"
        for i in range(500)
    ) + "\n"
    for i in range(12):
        yield f"tabs/tab-{i:02}.md", f"# 标签页 {i}\n\n" + ordinary


def manifest():
    return {
        "version": VERSION,
        "encoding": "UTF-8",
        "newline": "LF",
        "files": [
            {"path": name, "bytes": len(text.encode()),
             "sha256": hashlib.sha256(text.encode()).hexdigest()}
            for name, text in sources()
        ],
        "scenarios": {
            "ordinary": ["ordinary.md"],
            "long-paragraph": ["long-paragraph.md"],
            "large-document": ["large-document.md"],
            "formula-table": ["formula-table.md"],
            "multiple-tabs": [f"tabs/tab-{i:02}.md" for i in range(12)],
        },
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    # Exclusive creation protects both real vaults and previously edited samples.
    args.output.mkdir(parents=True, exist_ok=False)
    for name, text in sources():
        path = args.output / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(text.encode("utf-8"))
    data = manifest()
    (args.output / ".performance-corpus.json").write_text(
        json.dumps(data, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
    )
    print(json.dumps(data, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
