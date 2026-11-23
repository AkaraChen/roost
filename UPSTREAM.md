# 可以回馈上游的补丁

我们对 vendor 进来的 gc / gc_derive 做的改动（`vendor/`，说明见 [`vendor/README.md`](vendor/README.md)），
以及每一项能不能、该怎么提给上游。上游：<https://github.com/Manishearth/rust-gc>。

状态都是**还没提**。提之前请先确认上游 master 的情况没变（下面的结论基于 2026-11-23 的 master，
最新 commit `b7405c95`）。

## 1. `GcBox::from_box`：新 nightly 上 E0606（gc，`nightly` feature）——值得提

**现象。** 开 `nightly` feature，从 nightly-2026-08-27 起（2026-08-20 正常；到 2026-11-23 仍然如此）：

```
error[E0606]: casting `*mut dyn Trace` as `*mut GcBox<T>` is invalid
   --> gc/src/gc.rs:163:38
    |
163 |             let gcbox = set_data_ptr(value as *mut GcBox<T>, gcbox_addr);
    |                                      ^^^^^^^^^^^^^^^^^^^^^^
    = note: the pointers may have different metadata
```

上游 master 的 `gc/src/gc.rs` 同一行代码没变，所以 master 也编不过；issue 列表里没有相关报告。

**原因。** 不是 cast 规则变了，是类型推断变了。`from_box` 在 `T: Trace + Unsize<dyn Trace> + ?Sized`
下写的是 `let value = Box::into_raw(value);`，`Box::into_raw` 的类型参数靠推断。新 nightly 会用作用域里
的 `T: Unsize<dyn Trace>` 把实参 `Box<T>` 先 unsize 成 `Box<dyn Trace>`，于是 `value: *mut dyn Trace`，
后面 `value as *mut GcBox<T>` 就成了元数据不同的 cast。最小复现（不依赖 gc）：

```rust
#![feature(unsize)]
use std::marker::Unsize;
trait Tr {}
struct W<T: ?Sized> { h: u8, d: T }
pub fn f<T: Tr + Unsize<dyn Tr> + ?Sized>(b: Box<T>) -> *mut W<T> {
    let v = Box::into_raw(b); // 新 nightly 推成 *mut dyn Tr
    v as *mut W<T>            // E0606
}
```

`nightly-2026-08-20` 编得过，`nightly-2026-11-23` 报 E0606。

**补丁**（`vendor/gc/src/gc.rs`，一行，外加注释）：

```diff
-            let value = Box::into_raw(value);
+            let value: *mut T = Box::into_raw(value);
```

打补丁后 gc 自己的测试套件：nightly-2026-11-23 上 `--features nightly,derive` 和 `--features derive`
都全绿；旧的 nightly-2026-08-20 上 `--features nightly,derive` 也全绿（补丁向后兼容）。不开 `nightly`
时这个 impl 的 `T` 是 `Sized`，标注不改变任何东西。

**怎么提。** 给 rust-gc 提 PR：就是这一行，附上 E0606 报错和上面的复现。

**可选：给 rustc 报一下。** 这个推断变化让原本能编的代码编不过，可能是有意的（新 trait solver /
coercion 改动的副作用），也可能是回归。可以拿上面的最小复现去 rust-lang/rust 搜一下 / 报 issue，
在 `2026-08-20..2026-08-27` 之间二分到具体 PR（`cargo-bisect-rustc`）。我们没做这一步。

## 2. gc_derive 升级到 synstructure 0.13：消掉 `non_local_definitions` 警告——上游已修，只差发版

**现象。** gc_derive 0.5.0 用 synstructure 0.12，生成的 impl 包在 `const _DERIVE_gc_Trace_FOR_X: () = { ... }`
里，新编译器在**每个** `#[derive(Trace)]` / `#[derive(Finalize)]` 上都报 `non_local_definitions`。
spike 当时只能在 crate 级 `#![allow(non_local_definitions)]`。

**上游状态。** 已修：rust-gc PR #175（2024-06 合并，对应 issue #174）把依赖换成 synstructure 0.13，
但 crates.io 上一直没有发 gc_derive 的新版本。

**补丁。** 我们只移植了这个修复本身：

- `gc_derive/Cargo.toml`：去掉 `syn = "1.0"`，`synstructure = "0.12"` → `"0.13"`；
- `gc_derive/src/lib.rs`：`attr.path.is_ident(..)` → `attr.path().is_ident(..)`（syn 2）。

这和上游 master 上的 gc_derive 源码一致（除了 import 顺序）。

**怎么提。** 不需要 PR，需要的是**发版**：在 rust-gc 开 issue 请维护者发布 gc / gc_derive 新版本，
理由是 #175 修掉的警告和上面第 1 项。发版之后我们就能删掉 `vendor/`。

## 不提的改动

- 两个 `Cargo.toml` 换成上游手写的 manifest、改成路径依赖、加 `publish = false`：只是 vendor 需要。
- `vendor/rustfmt.toml`：只是不让我们的 `cargo fmt --all` 改上游代码。
