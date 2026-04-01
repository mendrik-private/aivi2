#!/usr/bin/env python3

from __future__ import annotations

import argparse
import dataclasses
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import Iterable


FENCE_RE = re.compile(r"(?ms)^```(?P<lang>[^\n`]*)\n(?P<body>.*?)^```[ \t]*$")
LIKELY_AIVI_RE = re.compile(
    r"(?m)^\s*(use|export|fun|value|signal|type|data|source|result|view|adapter|class|instance|domain)\b|"
    r"\|\>|T\|\>|F\|\>|\|\|\>|<match\b|<show\b|<each\b|<case\b"
)


@dataclasses.dataclass
class FenceBlock:
    markdown_path: Path
    line: int
    block_index: int
    language: str
    body: str

    @property
    def display_language(self) -> str:
        return self.language or "<unlabeled>"


@dataclasses.dataclass
class CommandResult:
    command: list[str]
    returncode: int
    stdout: str
    stderr: str

    @property
    def ok(self) -> bool:
        return self.returncode == 0


@dataclasses.dataclass
class BlockReport:
    block: FenceBlock
    formatted: str | None
    lex: CommandResult | None
    fmt: CommandResult | None
    check: CommandResult | None
    compile: CommandResult | None
    build: CommandResult | None

    @property
    def ok(self) -> bool:
        parts = [self.lex, self.fmt, self.check, self.compile, self.build]
        return all(part is None or part.ok for part in parts)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description=(
            "Extract fenced AIVI snippets from the VitePress manual and validate them with "
            "the local AIVI CLI."
        )
    )
    parser.add_argument(
        "--root",
        type=Path,
        default=Path("manual"),
        help="Manual root to scan (default: manual)",
    )
    parser.add_argument(
        "--aivi",
        type=Path,
        default=Path("target/debug/aivi"),
        help="Path to the AIVI CLI binary (default: target/debug/aivi)",
    )
    parser.add_argument(
        "--include-unlabeled",
        action="store_true",
        help="Also inspect unlabeled fences that look like AIVI snippets",
    )
    parser.add_argument(
        "--json",
        action="store_true",
        help="Emit machine-readable JSON instead of the human summary",
    )
    parser.add_argument(
        "--rewrite-formatted",
        action="store_true",
        help="Rewrite matched fenced blocks in place with `aivi fmt --stdin` output",
    )
    parser.add_argument(
        "--files",
        nargs="*",
        default=None,
        help="Optional markdown paths, relative to repo root, to limit the scan",
    )
    return parser.parse_args()


def repo_root() -> Path:
    return Path(__file__).resolve().parent.parent


def find_blocks(root: Path, include_unlabeled: bool, files: list[str] | None) -> list[FenceBlock]:
    if files:
        markdown_files = [repo_root() / file for file in files]
    else:
        markdown_files = sorted(root.rglob("*.md"))
    blocks: list[FenceBlock] = []
    for markdown_path in markdown_files:
        text = markdown_path.read_text(encoding="utf-8")
        block_index = 0
        for match in FENCE_RE.finditer(text):
            language = match.group("lang").strip()
            body = match.group("body")
            is_aivi = language == "aivi"
            is_unlabeled_aivi = include_unlabeled and language == "" and LIKELY_AIVI_RE.search(body)
            if not is_aivi and not is_unlabeled_aivi:
                continue
            block_index += 1
            line = text.count("\n", 0, match.start()) + 1
            blocks.append(
                FenceBlock(
                    markdown_path=markdown_path,
                    line=line,
                    block_index=block_index,
                    language=language,
                    body=body,
                )
            )
    return blocks


def run_command(command: list[str], cwd: Path) -> CommandResult:
    completed = subprocess.run(command, cwd=cwd, capture_output=True, text=True)
    return CommandResult(
        command=command,
        returncode=completed.returncode,
        stdout=completed.stdout,
        stderr=completed.stderr,
    )


def run_stdin_command(command: list[str], text: str, cwd: Path) -> CommandResult:
    completed = subprocess.run(command, cwd=cwd, input=text, capture_output=True, text=True)
    return CommandResult(
        command=command,
        returncode=completed.returncode,
        stdout=completed.stdout,
        stderr=completed.stderr,
    )


def check_block(block: FenceBlock, aivi: Path, cwd: Path, scratch: Path) -> BlockReport:
    relative = block.markdown_path.relative_to(cwd)
    safe_stem = str(relative).replace("/", "__")
    source_path = scratch / f"{safe_stem}__line_{block.line}__block_{block.block_index}.aivi"
    object_path = source_path.with_suffix(".o")
    source_path.write_text(block.body, encoding="utf-8")

    lex = run_command([str(aivi), "lex", str(source_path)], cwd)
    fmt_stdin = run_stdin_command([str(aivi), "fmt", "--stdin"], block.body, cwd)
    fmt = run_command([str(aivi), "fmt", "--check", str(source_path)], cwd)
    check = run_command([str(aivi), "check", str(source_path)], cwd)
    compile_result = None
    build_result = None
    needs_build = "@source" in block.body or re.search(r"<[A-Za-z/]", block.body) is not None
    if check.ok and not needs_build:
        compile_result = run_command(
            [str(aivi), "compile", str(source_path), "-o", str(object_path)],
            cwd,
        )
    elif check.ok and needs_build:
        bundle_path = scratch / f"{safe_stem}__line_{block.line}__block_{block.block_index}__bundle"
        build_result = run_command(
            [str(aivi), "build", str(source_path), "-o", str(bundle_path)],
            cwd,
        )
    return BlockReport(
        block=block,
        formatted=fmt_stdin.stdout if fmt_stdin.ok else None,
        lex=lex,
        fmt=fmt,
        check=check,
        compile=compile_result,
        build=build_result,
    )


def summarise_text(reports: Iterable[BlockReport], cwd: Path) -> str:
    grouped: dict[Path, list[BlockReport]] = {}
    for report in reports:
        grouped.setdefault(report.block.markdown_path, []).append(report)

    lines: list[str] = []
    failing = 0
    total = 0
    for path in sorted(grouped):
        file_reports = grouped[path]
        bad = [report for report in file_reports if not report.ok]
        if not bad:
            continue
        lines.append(f"\n=== {path.relative_to(cwd)} ===")
        for report in bad:
            total += 1
            failing += 1
            lines.append(
                f"  block {report.block.block_index} line {report.block.line} [{report.block.display_language}]"
            )
            for label, result in (
                ("lex", report.lex),
                ("fmt", report.fmt),
                ("check", report.check),
                ("compile", report.compile),
                ("build", report.build),
            ):
                if result is None or result.ok:
                    continue
                detail = (result.stderr or result.stdout).strip().splitlines()
                snippet = detail[0] if detail else "(no output)"
                lines.append(f"    {label}: {snippet}")
    if failing == 0:
        return "All audited manual AIVI code blocks passed lex/fmt/check/compile."
    return "\n".join(lines).lstrip()


def summarise_json(reports: Iterable[BlockReport], cwd: Path) -> str:
    payload = []
    for report in reports:
        payload.append(
            {
                "file": str(report.block.markdown_path.relative_to(cwd)),
                "line": report.block.line,
                "block": report.block.block_index,
                "language": report.block.language,
                "ok": report.ok,
                "formatted": report.formatted,
                "commands": {
                    key: None
                    if result is None
                    else {
                        "ok": result.ok,
                        "returncode": result.returncode,
                        "stderr": result.stderr,
                        "stdout": result.stdout,
                    }
                    for key, result in {
                        "lex": report.lex,
                        "fmt": report.fmt,
                        "check": report.check,
                        "compile": report.compile,
                        "build": report.build,
                    }.items()
                },
            }
        )
    return json.dumps(payload, indent=2)


def rewrite_blocks(reports: list[BlockReport]) -> None:
    reports_by_file: dict[Path, list[BlockReport]] = {}
    for report in reports:
        if report.formatted is None:
            continue
        reports_by_file.setdefault(report.block.markdown_path, []).append(report)

    for markdown_path, file_reports in reports_by_file.items():
        file_reports.sort(key=lambda report: report.block.line)
        original = markdown_path.read_text(encoding="utf-8")
        rebuilt: list[str] = []
        cursor = 0
        matches = list(FENCE_RE.finditer(original))
        target_map = {
            (report.block.line, report.block.language, report.block.block_index): report
            for report in file_reports
        }
        block_index = 0
        for match in matches:
            language = match.group("lang").strip()
            body = match.group("body")
            is_aivi = language == "aivi"
            is_unlabeled_aivi = language == "" and LIKELY_AIVI_RE.search(body)
            if is_aivi or is_unlabeled_aivi:
                block_index += 1
                line = original.count("\n", 0, match.start()) + 1
                key = (line, language, block_index)
                report = target_map.get(key)
                if report is not None and report.formatted is not None:
                    formatted = report.formatted
                    if not formatted.endswith("\n"):
                        formatted = f"{formatted}\n"
                    rebuilt.append(original[cursor:match.start("body")])
                    rebuilt.append(formatted)
                    cursor = match.end("body")
        rebuilt.append(original[cursor:])
        markdown_path.write_text("".join(rebuilt), encoding="utf-8")


def main() -> int:
    args = parse_args()
    cwd = repo_root()
    root = (cwd / args.root).resolve()
    aivi = (cwd / args.aivi).resolve()
    if not aivi.exists():
        print(f"AIVI binary does not exist: {aivi}", file=sys.stderr)
        return 2
    blocks = find_blocks(root, args.include_unlabeled, args.files)
    if not blocks:
        print("No matching fenced AIVI blocks found.")
        return 0
    scratch = Path(tempfile.mkdtemp(prefix="aivi-manual-audit-"))
    try:
        reports = [check_block(block, aivi, cwd, scratch) for block in blocks]
    finally:
        shutil.rmtree(scratch, ignore_errors=True)

    if args.rewrite_formatted:
        rewrite_blocks(reports)

    output = summarise_json(reports, cwd) if args.json else summarise_text(reports, cwd)
    print(output)
    return 0 if all(report.ok for report in reports) else 1


if __name__ == "__main__":
    raise SystemExit(main())
