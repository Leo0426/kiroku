#!/usr/bin/env python3
"""用 amane 的参考实现为其测试输入生成 golden 数据(阶段 3 差分测试的基准)。

    python3 tools/gen_amane_golden.py <amane 仓库路径>

amane 的 parsing/file_info.py 只依赖标准库;这里直接加载该文件并桩掉 pytest,
读取 tests/parsing/test_file_info.py 里的全部输入,输出 amane 的实际解析结果。
GPL-3.0:golden 数据派生自 amane 的测试输入,本项目同为 GPL-3.0-or-later。
"""
import importlib.util, json, re, sys, types
from pathlib import Path

root = Path(sys.argv[1])
spec = importlib.util.spec_from_file_location("fi", root / "src/amane/parsing/file_info.py")
fi = importlib.util.module_from_spec(spec)
sys.modules["fi"] = fi          # dataclass 需要模块可被查找
spec.loader.exec_module(fi)

# 桩:让测试文件在没有 pytest / pydantic 的环境里可导入
pt = types.ModuleType("pytest"); pt.mark = types.SimpleNamespace(parametrize=lambda *a, **k: (lambda f: f))
pt.fixture = lambda *a, **k: (lambda f: f)
sys.modules["pytest"] = pt
pkg = types.ModuleType("amane"); parsing = types.ModuleType("amane.parsing")
for n in ("ContentType", "Mosaic", "detect_cd", "extract_number", "infer_content_type", "is_amateur",
          "is_uncensored", "parse_file_info", "split_number"):
    setattr(parsing, n, getattr(fi, n))
sys.modules["amane"] = pkg; sys.modules["amane.parsing"] = parsing
ns = {}
src = (root / "tests/parsing/test_file_info.py").read_text()
exec(compile(src, "test_file_info.py", "exec"), ns)

def info(path):
    i = fi.parse_file_info(path)
    return {"number": i.number, "content_type": i.content_type.value, "prefix": i.prefix, "cd": i.cd,
            "has_subtitle": i.has_subtitle, "mosaic": i.mosaic.value if i.mosaic else None, "definition": i.definition}

# 只当占位用的真实编号前缀换成虚构前缀;规则表里的前缀(HEYZO、LUXU、SIRO 等)保持原样。
RENAME = {"MIDV": "MOVX", "SSIS": "SMPL", "SSNI": "SNPX"}
def rename(s):
    for a, b in RENAME.items():
        s = re.sub(rf"(?<![A-Za-z]){a}(?=[-_ ]?\d)", b, s)
    return s

paths = [rename(c.path) for c in ns["CASES"] if hasattr(c, "path")]
out = {
    "source": "sqzw-x/amane tests/parsing/test_file_info.py(占位编号已改名,见 RENAME)",
    "paths": [{"input": p, **info(p)} for p in paths],
    "extract_number": [{"input": rename(t), "number": fi.extract_number(rename(t))} for t, _ in ns["EXTRACT_NUMBER_CASES"]],
}
print(json.dumps(out, ensure_ascii=False, indent=1))
