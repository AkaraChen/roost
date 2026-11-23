# vendor/

| 目录 | 来源 | 上游 commit（`.cargo_vcs_info.json`） |
| --- | --- | --- |
| `gc/` | crates.io `gc` 0.5.1 | `292287d0adc537832713dad68d2eb71ef4d2bdd2` |
| `gc_derive/` | crates.io `gc_derive` 0.5.0 | `a70603225aa46be8570227c0fbc53a45bbd78c6e` |

上游仓库：<https://github.com/Manishearth/rust-gc>（MPL-2.0，许可证随文件保留）。

## 为什么 vendor

gc 0.5.1 的 `nightly` feature（`Gc<dyn Trait>` 靠它）从 nightly-2026-08-27 起编不过，
上游 master（截至 2026-11-23）也没修，crates.io 上也没有新版本。不 vendor 就只能永远钉在
nightly-2026-08-20。放进仓库而不是用 `[patch]` 指向 fork，是为了让 reviewer 在 PR 里直接看到改了什么。

## 怎么看改了什么

导入分两个 commit：

1. `vendor: import gc 0.5.1 and gc_derive 0.5.0 unmodified from crates.io`：`.crate` 包的原样内容
   （只删了 `.cargo-ok` 和 gc 自带的 `Cargo.lock`）。
2. 之后的 commit：我们的改动。`git diff <commit 1> -- vendor/` 就是全部补丁。

改动清单（细节和能否回馈上游见仓库根目录的 [`UPSTREAM.md`](../UPSTREAM.md)）：

| 文件 | 改动 |
| --- | --- |
| `gc/src/gc.rs` | `GcBox::from_box`：给 `Box::into_raw` 的结果加类型标注，修 E0606 |
| `gc_derive/src/lib.rs` | `attr.path` → `attr.path()`（syn 2 API） |
| `gc_derive/Cargo.toml` | `syn 1` + `synstructure 0.12` → `synstructure 0.13`（syn 2），消掉每个 `#[derive(Trace)]` 都报的 `non_local_definitions` 警告 |
| `*/Cargo.toml` | 换回上游手写的 manifest（`.crate` 里的 `Cargo.toml.orig`，而不是 cargo 规范化过的那份），`gc_derive` 走路径依赖，加 `publish = false` |
| `rustfmt.toml` | 新增，`disable_all_formatting`：`cargo fmt --all` 不要碰上游代码 |

## 在 workspace 里的位置

- 根 `Cargo.toml` 用 `[workspace.dependencies] gc = { path = "vendor/gc", ... }` 引用。
- `vendor/` 被 `exclude` 在 workspace 之外，当作第三方代码：我们的 `clippy -D warnings` 不检查它
  （上游代码有 `missing_safety_doc` 之类的 clippy 警告，不值得为此改上游代码）。
- 它自己的测试套件单独跑（CI 里也有这一步），`nightly` feature 开和不开都要绿：

  ```sh
  cargo test --manifest-path vendor/gc/Cargo.toml --features nightly,derive
  cargo test --manifest-path vendor/gc/Cargo.toml --features derive
  ```

  这样会在 `vendor/gc/` 下生成 `target/` 和 `Cargo.lock`，都已被 `vendor/gc/.gitignore` 忽略。

## 升级 / 撤掉

上游发了修好的版本（见 `UPSTREAM.md`）之后：删掉 `vendor/`，把根 `Cargo.toml` 里的 `gc` 改回
`version = "..."`，跑一遍 CI 的全部步骤。
