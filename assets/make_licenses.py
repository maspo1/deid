"""정보 창의 오픈소스 목록(assets/licenses.tsv) 생성.

Cargo 의존성 그래프에서 실행 파일에 실제로 들어가는(normal) 크레이트만 골라
Windows / macOS 대상별로 모은다. 의존성을 바꾼 뒤 다시 실행하고 다시 빌드한다.

    python assets/make_licenses.py
"""
import json
import os
import subprocess

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
TARGETS = {"x86_64-pc-windows-gnu": "Windows", "aarch64-apple-darwin": "macOS"}


def walk(meta):
    """루트에서 normal 의존성만 따라가며 도달하는 패키지 id 집합"""
    nodes = {n["id"]: n for n in meta["resolve"]["nodes"]}
    root = meta["resolve"]["root"]
    seen, stack = set(), [root]
    while stack:
        cur = stack.pop()
        if cur in seen:
            continue
        seen.add(cur)
        for d in nodes[cur]["deps"]:
            if any(k["kind"] is None for k in d["dep_kinds"]):
                stack.append(d["pkg"])
    seen.discard(root)
    return seen


def main():
    pkgs, used = {}, {}
    for triple, label in TARGETS.items():
        out = subprocess.run(
            ["cargo", "metadata", "--format-version", "1", "--filter-platform", triple],
            cwd=ROOT, capture_output=True, text=True, encoding="utf-8", check=True,
        ).stdout
        meta = json.loads(out)
        for p in meta["packages"]:
            pkgs[p["id"]] = p
        for pid in walk(meta):
            used.setdefault(pid, set()).add(label)

    rows = []
    for pid, where in used.items():
        p = pkgs[pid]
        usage = "공통" if len(where) == len(TARGETS) else ", ".join(sorted(where)) + " 전용"
        repo = p.get("repository") or p.get("homepage") or f"https://crates.io/crates/{p['name']}"
        lic = (p.get("license") or "파일 참조").replace("/", " OR ")
        rows.append((p["name"].lower(), p["name"], p["version"], lic, repo, usage))
    rows.sort()
    path = os.path.join(ROOT, "assets", "licenses.tsv")
    with open(path, "w", encoding="utf-8", newline="\n") as f:
        f.write("# name\tversion\tlicense\trepository\tusage  (make_licenses.py 로 생성)\n")
        for r in rows:
            f.write("\t".join(r[1:]) + "\n")
    print(f"{len(rows)}개 → {path}")


if __name__ == "__main__":
    main()
