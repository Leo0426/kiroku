//! 差分测试:amane 参考实现对同一批输入的解析结果(tools/gen_amane_golden.py 生成)。
//! 逐字段比较:番号、内容类型、前缀、分集、字幕、mosaic 版本标记、清晰度;另比较自由文本的 extract_number。

use kiroku_core::code::{extract_number, parse_file_info};
use serde_json::Value;

fn golden() -> Value {
    serde_json::from_str(include_str!("golden/amane_file_info.json")).unwrap()
}

fn js<T: serde::Serialize>(v: T) -> Value {
    serde_json::to_value(v).unwrap()
}

/// kiroku 有意偏离 amane 的字段:(输入, 字段, kiroku 取值)。只放确认过的扩展,不是兜底白名单。
const KIROKU_EXTENSIONS: &[(&str, &str, &str)] = &[
    // amane 解析番号时已剥掉空格分隔的 CD 标记,却没计入分集
    ("/media/SMPL-456 CD2.mp4", "cd", "2"),
];

#[test]
fn path_parsing_matches_amane() {
    let g = golden();
    let mut diffs = vec![];
    let mut total = 0;
    for e in g["paths"].as_array().unwrap() {
        let input = e["input"].as_str().unwrap();
        let i = parse_file_info(input);
        let got = serde_json::json!({
            "number": i.number, "content_type": i.content_type, "prefix": i.prefix, "cd": i.cd,
            "has_subtitle": i.has_subtitle, "mosaic": i.mosaic, "definition": i.definition,
        });
        total += 1;
        for k in ["number", "content_type", "prefix", "cd", "has_subtitle", "mosaic", "definition"] {
            if KIROKU_EXTENSIONS.iter().any(|(i, f, v)| *i == input && *f == k && got[k].to_string() == *v) {
                continue;
            }
            if got[k] != e[k] {
                diffs.push(format!("{input:<42} {k}: amane={} kiroku={}", e[k], got[k]));
            }
        }
    }
    assert!(total >= 200, "golden 数据异常: {total}");
    println!("字段差异 {} 处 / {total} 条路径", diffs.len());
    assert!(diffs.is_empty(), "\n{}", diffs.join("\n"));
}

#[test]
fn extract_number_matches_amane() {
    let g = golden();
    for e in g["extract_number"].as_array().unwrap() {
        let input = e["input"].as_str().unwrap();
        assert_eq!(js(extract_number(input)), e["number"], "输入 {input:?}");
    }
}
