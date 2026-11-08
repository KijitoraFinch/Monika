"""Exercise the bundled extensions in a temporary project (Python 3.9+)."""

import argparse
import json
from pathlib import Path
import subprocess
import tempfile


def demonstrate(binary, directory):
    def run(arguments, value=None, expected=0):
        process = subprocess.run(
            [str(binary), *arguments], cwd=directory,
            input="" if value is None else json.dumps(value),
            capture_output=True, text=True,
        )
        if process.returncode != expected:
            raise RuntimeError(f"{arguments}: {process.returncode}\n{process.stdout}\n{process.stderr}")
        return [json.loads(line) for line in process.stdout.splitlines()]

    def selector(extension, resource, query=None):
        return {"extension": extension, "resource": resource, "query": query}

    def read(extension, resource, query=None):
        return run(["read"], selector(extension, resource, query))[0]["region"]["content"]

    run(["init"])
    manifests = run(["extension", "list"])
    assert {item["name"] for item in manifests} == {
        "file", "http", "git", "text", "markdown", "html", "json", "jsonl",
        "yaml", "toml", "csv", "tsv", "code", "anchor", "assert",
    }
    (directory / "notes.md").write_text("# Plan\nKeep the source.\n## Detail\nUse a Capture.\n# Next\nLater.\n")
    (directory / "main.rs").write_text("fn run() { println!(\"ready\"); }\nfn other() {}\n")
    (directory / "data.json").write_text('{"answer":42,"other":0}\n')
    (directory / "data.yaml").write_text("answer: 42\nother: 0\n")
    (directory / "data.toml").write_text("answer = 42\nother = 0\n")
    (directory / "events.jsonl").write_text('{"ok":false}\n{"ok":true}\n')
    (directory / "rows.csv").write_text('name,status\nAlice,ok\nBob,skip\n')
    (directory / "rows.tsv").write_text('name\tstatus\nAlice\tok\nBob\tskip\n')
    (directory / "page.html").write_text('<article><p id="result">Ready</p><p>Next</p></article>')
    assert read("text", "notes.md", {"lines": [2, 2]}) == {"text": "Keep the source.\n"}
    section = read("markdown", "notes.md", {"section": "Plan"})["text"]
    assert "## Detail" in section and "# Next" not in section
    for extension in ("json", "yaml"):
        assert read(extension, "data." + extension, {"pointer": "/answer"}) == {"values": [42]}
    assert read("toml", "data.toml", {"pointer": "/answer"}) == {"values": [{"type": "integer", "value": 42}]}
    assert read("jsonl", "events.jsonl", {"lines": [2]}) == {"values": [{"ok": True}]}
    for extension in ("csv", "tsv"):
        value = read(extension, "rows." + extension, {
            "columns": ["name"], "where": [{"column": "status", "equals": "ok"}],
        })
        assert value == {"columns": ["name"], "rows": [["Alice"]]}
    assert read("html", "page.html", {"css": "#result"}) == {"html": ['<p id="result">Ready</p>']}
    assert read("code", "main.rs", {"symbol": "run"}) == {"text": 'fn run() { println!("ready"); }'}
    assert read("file", "main.rs")["size"] == (directory / "main.rs").stat().st_size

    # Cross-extension containment uses the same observed UTF-8 source.
    comparison = run(["compare"], {
        "left": selector("text", "main.rs"),
        "right": selector("code", "main.rs", {"symbol": "run"}),
    })
    assert comparison[0]["relation"] == "contains"
    for name, cases in {
        "documents": [selector("text", "notes.md"), selector("markdown", "notes.md", {"section": "Plan"}), selector("markdown", "notes.md", {"section": ["Plan", "Detail"]})],
        "json": [selector("json", "data.json"), selector("json", "data.json", {"pointer": "/answer"}), selector("json", "data.json", {"pointer": "/other"})],
        "csv": [selector("csv", "rows.csv"), selector("csv", "rows.csv", {"rows": [0]}), selector("csv", "rows.csv", {"rows": [0], "columns": ["name"]})],
        "html": [selector("html", "page.html"), selector("html", "page.html", {"css": "article"}), selector("html", "page.html", {"css": "#result"})],
    }.items():
        (directory / "cases.json").write_text(json.dumps(cases))
        report = run(["extension", "test", "--cases", "cases.json"])[0]
        assert report["passed"] and report["complete"], (name, report)

    # An anchor keeps a quoted Capture referenced by the Sidecar.
    link = {
        "id": "report-answer",
        "source": {"extension": "anchor", "query": "report:answer"},
        "target": selector("json", "data.json", {"pointer": "/answer"}),
        "annotation": {"expect": [{"endpoint": "target", "pointer": "/values/0", "equals": 42}]},
    }
    (directory / "monika.links.json").write_text(json.dumps({"version": 1, "links": [link]}))
    config_path = directory / "monika.json"
    config = json.loads(config_path.read_text())
    config["checks"] = [{"extension": "assert"}]
    config_path.write_text(json.dumps(config))
    run(["capture", "--link", "report-answer", "--endpoint", "both"])
    captured = run(["links"])[0]
    assert run(["check"], captured)[0]["valid"]
    (directory / "data.json").write_text('{"answer":42,"other":123}\n')
    assert run(["check"], captured)[0]["valid"]
    (directory / "data.json").write_text('{"answer":43,"other":123}\n')
    run(["check"], captured, expected=1)
    assert run(["read", "--at", "/target", "--captured"], captured)[0]["region"]["content"] == {"values": [42]}
    run(["gc"])
    assert run(["read", "--at", "/target", "--captured"], captured)[0]["region"]["content"] == {"values": [42]}
    config["resources"] = {"data.json": "refreshed"}
    config_path.write_text(json.dumps(config))
    run(["read"], link["target"], expected=2)
    run(["refresh", "data.json"])
    (directory / "data.json").write_text('{"answer":44}\n')
    assert run(["read"], link["target"])[0]["region"]["content"] == {"values": [43]}
    relocated = directory.with_name("relocated")
    directory.rename(relocated)
    directory = relocated
    assert run(["read"], link["target"])[0]["region"]["content"] == {"values": [43]}
    print("Verified bundled extensions, cross-extension comparison, Capture, checks, GC, refresh and relocation")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--monika", type=Path, required=True)
    args = parser.parse_args()
    with tempfile.TemporaryDirectory(prefix="monika-bundled-demo-") as temporary:
        workspace = Path(temporary) / "project"
        workspace.mkdir()
        demonstrate(args.monika.resolve(), workspace)
