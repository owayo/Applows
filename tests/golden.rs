//! ゴールデン (スナップショット) テスト。
//!
//! `tests/fixtures/smoke.aplo` の最終ポリグロット出力を固定し、意図しないコード生成の
//! 変化を検知する。ゴールデンの再生成は `UPDATE_GOLDEN=1 cargo test` で行う。

use applows::compile;
use std::path::Path;

#[test]
fn smoke_polyglot_golden() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/smoke.aplo");
    let golden = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/smoke.polyglot.golden");

    let src = std::fs::read_to_string(&fixture).expect("fixture 読み込み");
    let result = compile(&src).expect("smoke.aplo はコンパイルできるはず");

    if std::env::var("UPDATE_GOLDEN").is_ok() {
        std::fs::create_dir_all(golden.parent().unwrap()).unwrap();
        std::fs::write(&golden, result.output.as_bytes()).unwrap();
        eprintln!("golden 更新: {}", golden.display());
        return;
    }

    let expected = std::fs::read_to_string(&golden)
        .expect("golden が無い。UPDATE_GOLDEN=1 cargo test で生成する");
    assert_eq!(
        result.output, expected,
        "生成物が golden と一致しない。意図的な変更なら UPDATE_GOLDEN=1 cargo test で更新する"
    );
}
