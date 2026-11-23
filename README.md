# roost

An experiment: a Java-like object model for Rust — classes, field inheritance, overridden
methods dispatched through a vtable, and instanceof-style downcasting — on top of a tracing
garbage collector, all behind macros.

Status: **spike**. Nothing here is an API yet. The macros are deliberately crude `macro_rules!`.

```sh
cargo run    # walks through the five demos
cargo test   # asserts them
```

## 工具链

- `gc = { version = "0.5.1", features = ["derive", "nightly"] }`。`Gc<dyn Animal>` 只在 gc 的
  `nightly` feature 下存在（`CoerceUnsized` / `DispatchFromDyn` / `Unsize`）。
- `rust-toolchain.toml` 钉在 **`nightly-2026-08-20`**，这不是随便挑的：gc 0.5.1 的 `nightly`
  feature 在新 nightly 上**编译不过**（gc 自己的 `GcBox::from_box` 里
  `error[E0606]: casting *mut dyn Trace as *mut GcBox<T> is invalid`）。二分结果：
  `2026-08-20` 能编，`2026-08-27` 起（含 `2026-11-20`）编不过。
- 我们自己额外开了 `#![feature(arbitrary_self_types)]`（为了 `self: Gc<Self>`，见下文问题 2）；
  测试里额外用了 `ptr_metadata`（只为读出 vtable 指针做断言）。
- CI 会检查 `rustc -V` 确实是 nightly。

## 结构

| 文件 | 内容 |
| --- | --- |
| `src/object.rs` | `Object` / `Class` trait、`GcExt::{is, cast}`，以及 `base_class!` / `derived_class!` 两个宏 |
| `src/zoo.rs` | 用宏写出来的 `Animal`（根类）→ `Dog` |
| `src/probe.rs` | 线程本地的 drop / finalize 计数器 |
| `tests/spike.rs` | 五件事的断言 + GcCell 冲突的证据 |
| `src/main.rs` | `cargo run` 演示 |

`base_class!` 对 `Animal` 生成：

- `struct AnimalBase { name, friend, first_words, probe }`：根类的字段，本身也可实例化；
- `trait Animal: Object`：vtable。引用类型是 `Gc<dyn Animal>`。每个虚方法都有默认实现，转发到基类实现；
- `mod animal_super`：每个虚方法对应一个 `fn speak<T: ?Sized + Animal>(this: &T)`，也就是基类那一份，
  子类里的 `super.speak()` 写成 `animal_super::speak(this)`；
- `fn animal(&self) -> &AnimalBase`（通过 vtable 访问继承来的字段）和
  `fn as_animal(self: Gc<Self>) -> Gc<dyn Animal>`（把 `this` 变成 GC 指针）。

`derived_class!` 生成 `struct Dog { pub base: AnimalBase, breed }`（字段包含式继承），并且
`impl Animal for Dog`：被覆盖的方法用用户写的方法体，没覆盖的沿用默认实现。

## 五件事，以及证据在哪

| # | 内容 | 测试 |
| --- | --- | --- |
| 1 | Animal → Dog，字段包含 + vtable，`Gc<dyn Animal>` | `one_inheritance_by_field_containment` |
| 2 | Dog 覆盖 `speak`，经 `Gc<dyn Animal>` 动态派发 | `two_override_dispatches_through_the_vtable` |
| 3 | `super` 调用 | `three_super_call_reaches_the_base_implementation`，`three_b_virtual_call_in_base_constructor_is_not_virtual` |
| 4 | `is::<Dog>()` / `cast::<Dog>()`，失败返回 `None` | `four_instanceof_and_checked_downcast`，`four_b_cast_keeps_the_object_alive_and_rooted` |
| 5 | 互相引用的对象环被回收 | `five_cycle_is_collected`，`five_b_self_cycle_is_collected_and_reachable_cycles_are_not` |
| + | 相等就是 `Gc::ptr_eq`（类上根本没有 `PartialEq`） | `identity_is_ptr_eq` |

**派发确实走 vtable。** `speak_via_vtable(&Gc<dyn Animal>)` 是一个非泛型、`#[inline(never)]` 的函数，
只编译一份；对象的具体类型由 `black_box` 在运行期决定。同一份机器码对 Dog 返回 Dog 的文本、对
AnimalBase 返回基类的文本，只能是经过 vtable。测试还断言了 `Gc<dyn Animal>` 是两个指针宽、两种对象的
vtable 指针（`ptr::metadata`）不同，且 vtable 里记录的 `size_of` 分别等于 `Dog` / `AnimalBase`。
release 构建里这个函数的反汇编，最后一条就是通过 vtable 槽位的间接尾调用：

```
roost::speak_via_vtable:
   mov    %rdx,%r15           ; rdx = vtable 指针
   ...
   mov    %r15,%rcx
   ...
   jmp    *0x68(%rcx)         ; 跳到 vtable 里 speak 的槽位
```

**环回收的断言是咬得住的。** `five_cycle_is_collected` 先断言丢掉外部引用后 `dropped == 0`
（只靠引用计数，这个环会永远泄漏），再 `force_collect()`，然后断言 `finalized == 2 && dropped == 2`。
`dropped` 来自嵌在每个对象里的 `DropProbe` 的 `Drop`，也就是内存真的被释放了。
测试里的 `collect()` 会看环境变量 `ROOST_SKIP_COLLECT`，设了就什么都不做：

```sh
ROOST_SKIP_COLLECT=1 cargo test --test spike five_   # 两个环测试都失败
```

CI 里有一步专门跑这条命令，**要求它失败**。直接把 `collect();` 那一行删掉也验证过，同样失败
（`both objects finalized: left 0, right 2`）。

## 实践下来真正难的地方

### 1. 字段可变性：GcCell 的 borrow 冲突多容易撞上？撞上 panic 能接受吗？

每个可变字段各用一个 `GcCell`（`name: GcCell<String>`、`friend: GcCell<Option<Gc<dyn Animal>>>`）。
写这个 spike 的时候我没有*意外*撞上冲突；测试里的两个 panic 是我刻意构造的，但写法都是最顺手的那种：

- `rename_holding_borrow`：`let mut name = this.animal().name.borrow_mut(); ...; this.on_renamed(&old)`。
  对 `AnimalBase` 完全没问题（基类的 `on_renamed` 什么都不做），**一旦 Dog 覆盖了 `on_renamed`
  去读 `name`，就 panic**（`GcCell<T> already mutably borrowed`）。这是最难受的一点：冲突取决于
  *运行期的动态类型*，基类自己的测试永远测不出来，等于「脆弱基类」问题 + 运行期借用检查叠在一起。
  虚方法本来就是「调出去的代码不归我管」，而 guard 跨调用存活时，调出去的代码一碰同一字段就炸。
  见 `borrow_conflict_does_not_show_up_in_the_base_class` / `borrow_conflict_appears_only_once_a_subclass_overrides`。
- `borrow_conflict_reading_friend_while_befriending`：拿着 `friend.borrow()` 做判断，同时调用会改
  `friend` 的方法 → `GcCell<T> already borrowed`。
- `befriend` 之所以 `a.befriend(a)` 不炸，只是因为每个 `borrow_mut()` 都是语句内的临时值；
  如果把 guard 绑到 `let` 上，自己和自己交朋友就会 panic。

panic 的位置报告：gc 的 `borrow` 带 `#[track_caller]`，panic 指向**第二次**借用的那一行
（`src/zoo.rs` 里 `Dog::on_renamed`），而不是一直拿着 `borrow_mut` 的那一帧。要找到真正的「持有者」得开
`RUST_BACKTRACE=1`，在栈里往上翻到 `animal_super::rename_holding_borrow::<Dog>`。

能不能接受：作为 spike，能定位、能修（把 guard 的作用域缩到调用之前，见 `rename`）。作为「像 Java
一样写类」的体验，不太能接受：Java 用户默认字段随便读写，这里冲突是运行期的，还依赖动态类型。
下一棒建议让宏生成 `get_name() -> String` / `set_name(v)` 这类**不把 guard 交给用户**的访问器，
使借用永远不跨越方法调用；需要原地改大字段的场景再单独开口子。

### 2. `this` 的类型：最后写成什么样？`&self` 和 `this: &Gc<dyn X>` 哪个顺手？

最终签名：

- 绝大多数虚方法：vtable 里是 `fn speak(&self) -> String`。最顺手，`Gc<dyn Animal>` 上直接 `.speak()`。
- 需要把 `this` 作为 GC 指针存起来的方法（`befriend` 要把自己塞进对方的 `friend` 字段）：
  `fn befriend(self: Gc<Self>, other: Gc<dyn Animal>)`。代价：
  - 要再开一个 nightly feature `arbitrary_self_types`；
  - 按值接收，调用方得写 `a.clone().befriend(b.clone())`；
  - `self: &Gc<Self>` 试过，**不行**：`E0038 the trait Animal is not dyn compatible ... self parameter cannot be dispatched on`；
  - 在基类实现里 `Self: ?Sized`，`Gc<Self>` 没法 unsize 成 `Gc<dyn Animal>`，所以宏必须给每个类生成一个
    vtable 方法 `as_animal(self: Gc<Self>) -> Gc<dyn Animal>`。
- `this: &Gc<dyn X>` 当普通参数：不是接收者，就不可能是虚方法，只能作为 `&self` 之外的*额外*参数传，
  调用处要写 `a.foo(&a)`。spike 里没用这种形式。
- 方法体里用户写的是 `this`，不是 `self`（见问题 3：基类实现被展开成泛型自由函数
  `fn speak<T: ?Sized + Animal>(this: &T)`，自由函数不能有 `self` 参数）。覆盖方法里宏做
  `let this = self;`，两边写法一致。于是基类方法体里 `this` 的类型是 `&T`，Dog 的覆盖方法里是 `&Dog`。

结论：`&self` 明显更顺手，`Gc<Self>` 只在「this 要逃逸出去」的时候用；宏里这两种接收者只能拆成
`methods { }` / `gc_methods { }` 两段，因为 `macro_rules!` 的重复块里没法在 `&this` 和 `this` 之间二选一。

### 3. super 调用和「构造器里调用被子类覆盖的方法」：宏藏住了多少，还剩哪些陷阱？

宏藏住了：

- vtable 槽位的默认值（没覆盖的方法自动用基类实现）；
- super：`animal_super::speak(this)`。基类实现对 `T: ?Sized + Animal` 泛型，所以基类方法体里的
  `this.speak()` 依然落到子类：`animal_super::describe(dog)` 返回的是 `[Dog] ... Woof!`（有测试）。
- 析构链：Dog 派生出的 `finalize_glue` 会递归到 `base: AnimalBase` 字段，所以基类的 `Finalize`
  对 Dog 也会跑，像基类析构函数一样，不用手写。

还剩的陷阱（都是实际碰到的）：

- **第一版宏就踩了 super 的语义坑。** 一开始我把基类方法体原样贴进一个辅助 trait 的默认方法里
  （这样 `self` 能照抄）。结果基类方法体里的 `self.speak()` 报 `E0034 multiple applicable items`：
  辅助 trait 和 `Animal` 都有 `speak`。更糟的是，消掉歧义的话，它会静态绑定到*基类*的 `speak`，
  虚派发就悄悄丢了。最后改成泛型自由函数 + 用户写 `this`。
- **super 不是语法，是一个按类命名的模块路径**（`animal_super::`）。只有两层时够用；三层
  （Puppy → Dog → Animal）时 Dog 的覆盖方法也得生成一份 `dog_super::*`，Dog 也要有自己的 trait
  （`dyn Dog`，靠 trait upcasting 当 `dyn Animal` 用）。**这个 spike 没覆盖到三层。**
- **构造器里的虚调用不是虚的。** `Gc::new` 之前根本没有 `Gc<dyn Animal>`，基类构造器手里只有一个
  `AnimalBase` 值，所以 `AnimalBase::new` 里调 `speak()` 是静态调用基类版本，即使正在构造的是 Dog：
  `dog.animal().first_words == "Rex makes a sound"`，而不是 Dog 的 `"... Woof!"`
  （`three_b_virtual_call_in_base_constructor_is_not_virtual`）。这是 C++ 语义，不是 Java 语义：
  不会读到子类未初始化的字段，但「Java 里能工作的模板方法写法」在这里会静默地得到基类结果，没有任何报错。
  要在构造后做虚调用只能两段式（`Gc::new` 之后再调一个 `init` 虚方法），宏没做，**这个 spike 没覆盖到**。
- **`is` 和 `cast` 不对称。** `dog.is::<AnimalBase>()` 为真，但 `dog.cast::<AnimalBase>()` 是 `None`：
  字段包含式继承下，`Gc<AnimalBase>` 没法指向 Dog 分配块中间的那一段。向上转只能走 unsize
  （`Gc<Dog>` → `Gc<dyn Animal>`）；向下转只能转到*精确*的运行期类。转到非叶子类（`Gc<dyn Dog>`）
  需要每个非叶子类有自己的 trait + 运行期把 vtable 换掉，没做。
- 类上不能写 `Drop`：`#[derive(Trace)]` 自己生成了 `Drop`。要观察释放只能塞一个 probe 字段，或用 `Finalize`。

### 4. 报错可读性：宏生成的代码出错时指向哪里，能用吗？

我在 `zoo.rs` 里故意制造了这些错误，记下 rustc 指向哪里：

| 改动 | 报错 | 指向 | 可用吗 |
| --- | --- | --- | --- |
| 覆盖方法体返回 `42` | E0308 expected `String` | 用户那一行，还标出了 `-> String` | 好 |
| 基类方法体里 `this.speek()` | E0599 no method `speek` for `&T` + 建议 `speak` | 用户那一行 | 好，但出现了用户没写过的泛型 `&T` |
| 覆盖方法参数类型和基类不一致 | E0053 incompatible type for trait | 两处用户代码：覆盖处和基类声明处 | 很好 |
| 覆盖方法名拼错 `on_renamd` | E0407 not a member of trait | 主位置在 `object.rs` 宏内部，用户那行只出现在 `help: similar name` 里 | 勉强 |
| 字段类型没实现 `Trace`（`Cell<u32>`） | E0277 `Cell<u32>: Trace` not satisfied | `object.rs` 里的 `#[derive(::gc::Trace)]`，「in this macro invocation」框住整个类 | 差：得自己找是哪个字段 |
| 方法体里写 `self` 而不是 `this` | E0424 `self` not available，附「a macro invocation can only access identifiers it receives from parameters」 | 用户那一行 + 宏内部 | 知道宏怎么展开才看得懂 |
| 漏写 `gc_overrides {}` 段 | `no rules expected }` / `while trying to match gc_overrides` | 用户的 `}` + 宏规则 | 能定位，信息很晦涩 |
| `gc_overrides` 里写成 `&this` | `no rules expected &` / `while trying to match $gthis:ident` | 用户那一行 | 能定位，信息晦涩 |

规律：`macro_rules!` 原样传递用户的 token，所以**方法体内部**的错误指向用户代码，体验接近手写；
**结构性**的错误（方法名、字段类型、宏语法）落在宏定义里或者是 `no rules expected`，要懂宏才看得懂。
另外两个实际碰到的：基类方法里没用到的参数，`unused_variables` 警告指向 `object.rs` 而不是用户的方法
（已在宏里 `allow` 掉）；`rustfmt` 不格式化 `base_class! { ... }` 里面的内容。

想把结构性错误挪回用户代码上，得换成 proc macro（`syn` 解析、`quote_spanned!`、自己 emit
`compile_error!`）。这一棒没做。

### 其他实际踩到的

- **gc 0.5.1 + nightly 已经在腐烂。** 它的 `nightly` feature 在 2026-08-27 之后的 nightly 上编译不过（见上文工具链）。
  我们只能钉在旧 nightly 上，想升 nightly 就得 fork / patch gc。
- `gc_derive 0.5.0` 的 `#[derive(Trace)]` 在新编译器上每用一次都报 `non_local_definitions` 警告，
  只能在 crate 级 `allow`。
- 宏写 `Gc<dyn Trait>` 的默认方法时 `Self: ?Sized` 处处碍事（不能 unsize、不能 `&Self as &dyn Trait`），
  只能给每个类生成小的转换方法（`animal()`、`as_animal()`）放进 vtable。

## 下一棒建议

1. 先决定 gc 依赖怎么办：fork 修掉 `GcBox::from_box` 里的指针转换，或者向上游提；否则会一直卡在 `nightly-2026-08-20`。
2. 换 proc macro：结构性报错指向用户代码、语法更自然（比如对 `impl` 块加属性），并且能做三层及以上继承（非叶子类生成自己的 trait 和 `xxx_super` 模块）。
3. 字段访问由宏生成 getter / setter，不把 `GcCell` guard 交给用户，从根上避免问题 1 那种跨虚调用的借用冲突。
4. 设计两段式构造（`Gc::new` 之后再调虚的 `init`），把构造期的语义定下来写进文档。
5. 支持 cast 到非叶子类（`Gc<dyn Dog>`）。
