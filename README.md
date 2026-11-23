# roost

**用 Java 的写法在 Rust 里写类**：`class`、`extends`、`implements`、构造器、`super`、`@Override`、
`abstract`、`final`、`static`、`toString` / `equals` / `hashCode`、`instanceof`，对象由垃圾回收器管理，
互相引用的对象环也能回收。全部由一个 `class!` 宏完成。

给熟悉 Java、刚上手 Rust 的人：先看下面的 30 秒上手和对照表，再读 [教程](docs/tutorial.md)。
实现细节在 [设计说明](docs/design.md)。

## 30 秒上手

1. 在项目根目录放 `rust-toolchain.toml`（roost 需要 nightly，见[工具链](#工具链nightly)）：

   ```toml
   [toolchain]
   channel = "nightly-2026-11-23"
   components = ["rustfmt", "clippy"]
   ```

2. `Cargo.toml` 里加依赖：

   ```toml
   [dependencies]
   roost = { git = "https://github.com/AkaraChen/roost" }
   ```

3. `src/main.rs`：

```rust
use roost::prelude::*;

class! {
    public interface Pet {
        fn name() -> String;
        default fn greet() -> String { format!("Hi, I'm {}", this.name()) }
    }

    public abstract class Animal {
        protected name: String;

        public Animal(name: String) { this.name = name; }

        public abstract fn sound() -> String;

        public fn speak() -> String { format!("{} says {}", this.name, this.sound()) }
    }

    public class Dog extends Animal implements Pet {
        public Dog(name: String) { super(name); }

        @Override public fn sound() -> String { "woof".to_string() }
        @Override public fn name() -> String { this.name }
    }
}

#[roost::main]
fn main() {
    let zoo: Vec<Ref<Animal>> = vec![new!(Dog("Rex")), new!(Dog("Ace"))];
    for a in &zoo {
        println!("{}", a.speak());                // Rex says woof
    }
    if let Some(p) = zoo[0].cast::<Pet>() {       // instanceof + 强制转换
        println!("{}", p.greet());                // Hi, I'm Rex
    }
}
```

`cargo run`。你的代码里**不需要**写任何 `#![feature(..)]`。

## Java → roost 对照表

| Java | roost | 备注 |
| --- | --- | --- |
| `public class Dog extends Animal implements Pet { .. }` | 一样，写在 `class! { .. }` 里 | 一个继承树要写在同一个 `class!` 里 |
| `private String name;` | `private name: String;` | 名字在前，类型在后 |
| `private int legs = 4;` | `private legs: i32 = 4;` | 基本类型：`int`→`i32`，`long`→`i64`，`double`→`f64`，`boolean`→`bool` |
| `public Dog(String name) { super(name); }` | `public Dog(name: String) { super(name); }` | 每个类最多一个构造器 |
| `public String speak() { .. }` | `public fn speak() -> String { .. }` | 方法以 `fn` 开头 |
| `public void run() { .. }` | `public fn run() { .. }` | 不写 `->` 就是 `void` |
| `abstract String sound();` | `public abstract fn sound() -> String;` | |
| `@Override` | `@Override` | **必须写**（Java 里可选） |
| `this.name`、`this.speak()` | `this.name`、`this.speak()` | `this.` 不能省略；不要写 `self` |
| `super.speak()` | `super.speak()` | |
| `new Dog("Rex")` | `new!(Dog("Rex"))` 或 `Dog::new("Rex")` | `String` 参数直接传 `"Rex"` |
| `Animal a = new Dog("Rex");` | `let a: Ref<Animal> = new!(Dog("Rex"));` | `Ref<T>` 就是 Java 的引用 |
| `(Animal) dog`（向上转型） | 隐式；或 `dog.upcast::<Animal>()`、`new!(Dog(..)) as Ref<Animal>` | |
| `a instanceof Dog` | `a.instanceof::<Dog>()` | 类和接口都行 |
| `(Dog) a` | `a.cast::<Dog>()` → `Option<Ref<Dog>>` | 失败是 `None`，不抛异常 |
| `a == b`（同一个对象） | `Ref::ptr_eq(&a, &b)` | `Ref` 没有 `==` |
| `a.equals(b)` | `a.equals(b.clone())` | |
| `toString()` / `hashCode()` | `to_string()` / `hash_code()` | 用 `@Override` 覆盖；`println!("{a}")` 调 `to_string()` |
| `Object o` | `Ref<Object>` | 任何 `Ref` 都能隐式转过去 |
| `interface Pet extends Named { .. }` | 一样 | 方法、`default fn`、`static fn` |
| `final class` / `final` 方法 | `final class` / `final fn` | `final` 字段不支持，常量用 Rust 的 `const` |
| `static int count;` | `static count: i32;` | 类里写 `Animal::count` / `Self::count` |
| `Animal.create()` | `Animal::create()` | |
| `null` | `None`（字段类型写成 `Option<Ref<Dog>>`） | 没有 `null` |
| `public static void main(String[] args)` | `#[roost::main] fn main()` | 参数用 `std::env::args()` |
| `throws Exception` / 未捕获异常 | `fn main() -> Result<(), E>`，返回 `Err` 时打印 `Exception in thread "main" ..` | 退出码 1 |
| `System.gc()` | `roost::gc::collect()` | 一般不需要手动调 |
| `List<Animal>` | `Vec<Ref<Animal>>` | 容器用 Rust 标准库的 |

## 限制与差异

**和 Java 行为不同的地方**

- **读字段读到的是副本**。对 `i32`、`String`、`Ref<..>` 和 Java 没区别；对 `Vec` / `HashMap` 这类容器，
  `this.items.push(x)` 会编译失败（改副本会丢失修改）。要改就读出来、改、写回：
  `let mut v = this.items; v.push(x); this.items = v;`。
- **字段只能通过 `this.f` 访问**，即使是 `public` 字段，类外也访问不了，请写 getter。别的对象的字段
  （`other.name`）同样不能直接读，调它的 getter。
- **没有 `null`**：可空的引用是 `Option<Ref<X>>`。没有初始值的字段取 `Default::default()`（0、`false`、`""`、空容器），
  所以 `Ref<X>` 类型的字段必须是 `Option<Ref<X>>`。
- **没有重载**：每个类最多一个构造器，方法不能同名。多种构造方式请用 `static fn` 工厂方法。
- **`@Override` 必须写**，覆盖父类方法、实现接口方法、覆盖 `to_string` / `equals` / `hash_code` 都是。
- **静态字段每个线程一份**（垃圾回收器是线程本地的），只能在类里访问；初值在该线程第一次访问时求值。
- **方法体里的 `this` 不能存起来**（它不是 `Ref`），对象之间的引用要从外面建：`a.befriend(b.clone())`。
- **字段初始化器**（`legs: i32 = 4`）在分配对象时求值，不能用 `this`。
- 访问修饰符：类、构造器、静态方法、字段按 Java 的规则检查；但虚方法都在 Rust trait 里，**`protected` 和包级的
  虚方法实际上和 `public` 一样可见**。`private` 方法不是虚方法。
- 接口：不支持常量和 `private` 方法；子接口不能重新声明父接口的方法；不支持 `X.super.m()`；
  同名方法在几处声明的签名必须完全一致（不支持协变返回类型）。
- `abstract fn` 不能「重新抽象」一个继承来的具体方法。

**目前不支持**

- **一个继承树必须写在同一个 `class!` 块里**（不能 `extends` 别的 `class!` 块、别的模块或别的 crate 里的类）。
- 泛型类（`class Box<T>`）、泛型方法、`async` 方法。
- `final` 字段、`static final` 常量（用 Rust 的 `const`）、内部类、匿名类、枚举类、注解（除了 `@Override`）。
- 异常的 `try` / `catch`：可失败的操作用 Rust 的 `Result` 和 `?`。
- 多线程共享对象：`Ref` 不是 `Send`，每个线程有自己的堆。

## 工具链：nightly

- roost 需要 **nightly** Rust，仓库钉在 **`nightly-2026-11-23`**（`rust-toolchain.toml`），fmt / clippy / test / Miri
  都在它上面验证过。你的项目用同一个版本最稳妥（上面 30 秒上手里的 `rust-toolchain.toml`）。
- 原因：`Ref<Dog>` → `Ref<Animal>` 这种隐式转换和 trait object 上的方法派发要用到还没稳定的
  `CoerceUnsized` / `DispatchFromDyn` / `Unsize`；`class!` 的报错用到 nightly 的 `proc_macro::Diagnostic`。
  这些 feature 都只在 roost 自己的 crate 里开，**你的代码不需要任何 `#![feature]`**。
- 垃圾回收器是 vendor 进来并打过补丁的 gc 0.5.1（见 [`vendor/README.md`](vendor/README.md)）。

## 常见编译错误

roost 的报错尽量指向你写错的那个词，并给一行 Java 用户看得懂的 `help:`。下面每一条在
[`roost/tests/compile_fail/`](roost/tests/compile_fail/)（`#[roost::main]` 的在
[`roost-main-tests/tests/ui/`](roost-main-tests/tests/ui/)）都有一个用例和完整的报错快照（`.stderr`）。

| 你写了 | 报错 | 提示 / 怎么改 | 用例 |
| --- | --- | --- | --- |
| `extends Animl`（拼错） | unknown superclass `Animl` | did you mean `Animal`? | `unknown_superclass` |
| `pubilc fn bark()` | unknown modifier `pubilc` | did you mean `public`? | `misspelled_access_modifier` |
| `public clas Dog` | unknown keyword `clas` | did you mean `class`? | `misspelled_class_keyword` |
| `pub fn bark()` | `pub` is Rust's modifier | 写 `public` / `protected` / `private` | `rust_pub_modifier` |
| `private String name;` | Java-style field declaration | `name: String;` | `java_style_field` |
| `public void bark() { .. }` | Java-style method declaration | `fn bark(..) { .. }` | `java_style_method` |
| 字段后漏了 `;` | missing `;` after field `name` | `name: String;` | `missing_semicolon_after_field` |
| `this.nmae` | class `Dog` has no field `nmae` | did you mean `this.name`? | `unknown_field` |
| 子类里读父类的 `private` 字段 | field `name` is private to class `Animal` | 改成 `protected` | `private_field_in_subclass` |
| 方法体里写 `self.name` | use `this`, not `self` | `this.field` / `this.method()` | `self_in_method_body` |
| `this.friend = null;` | there is no `null` in roost | 字段用 `Option<Ref<..>>`，写 `None` | `null_field` |
| `static fn` 里用 `this` | there is no `this` in a static method | 去掉 `static`，或把对象作为参数传入 | `this_in_static_method` |
| `super(..)` 不在第一行 | `super(...)` must be the first statement | 移到构造器开头 | `super_call_not_first` |
| 父类构造器有参数，子类没写 `super(..)` | `Animal` has no no-arg constructor | 构造器以 `super(..);` 开头 | `implicit_super_without_noarg_ctor` |
| 构造器名拼错 `Dgo(..)` | neither the constructor of `Dog` nor a method | did you mean `Dog`? | `constructor_name_mismatch` |
| 写两个构造器 | overloading is not supported | 一个构造器 + `static fn` 工厂方法 | `constructor_overloading` |
| `extends Animal, Pet` | a class can extend only one class | 把 `Pet` 改成接口，`implements Pet` | `multiple_inheritance` |
| `class Cat extends Pet`（`Pet` 是接口） | `Pet` is an interface, not a class | `implements Pet` | `implements_a_class` |
| 覆盖时漏写 `@Override` | overrides `Animal.speak` but is not marked @Override | 加上 `@Override` | `override_without_annotation` |
| `@Override` 的方法名拼错 | marked @Override but does not override | did you mean `speak`? | `override_nothing` |
| 覆盖方法的参数类型和父类不同 | E0053 incompatible type for trait（同时指向父类的声明） | 改成父类的类型 | `override_signature_mismatch` |
| 实现接口方法时返回类型不同 | E0053 | 改成接口的类型 | `interface_method_wrong_signature` |
| 接口和父类声明了同名但签名不同的方法 | must have the same signature | 让签名一致 | `conflicting_declarations` |
| 实现接口方法漏写 `@Override` | implements `Pet.name` but is not marked @Override | 加上 `@Override` | `interface_impl_without_override` |
| 具体类没实现抽象方法 | class `Square` is not abstract and does not implement `Shape.area` | 实现它，或声明 `abstract class` | `missing_abstract_method` |
| 没实现接口方法 | does not implement `Pet.owner` | 同上 | `missing_interface_method` |
| 普通类里写 `abstract fn` | abstract method in class which is not abstract | `abstract class`，或给方法体 | `abstract_method_in_concrete_class` |
| `new!(Shape())`，`Shape` 是抽象类 | cannot instantiate abstract class `Shape` | 创建一个具体子类 | `new_abstract_class` |
| 继承 `final` 类 | cannot inherit from final class | 去掉 `final` | `extends_final_class` |
| 覆盖 `final` 方法 | cannot override final method | 去掉 `final` 或改名 | `override_final_method` |
| 字段类型不支持垃圾回收 | `Collar` does not implement `Trace` | 给它 `#[derive(Trace, Finalize)]` | `field_not_trace` |
| `this.items.push(x)` | cannot borrow data in dereference of `ReadOnlyCopyOfField<..>` as mutable | 读出来、改、写回 | `mutating_method_on_field` |
| `new!(Dog)` | missing the constructor arguments | `new!(Dog())` | `new_macro_java_syntax` |
| `new!(new Dog("Rex"))` | `new!` already means `new` | `new!(Dog("Rex"))` | `new_keyword_inside_new_macro` |
| `new!(Dog::new("Rex"))` | `new!` calls the constructor `::new` itself | `new!(Dog("Rex"))` | `new_macro_with_new_call` |
| `new!(Dog(42))`，参数应为 `String` | `String: From<{integer}>` is not satisfied（指向 `42`） | 传字符串 | `constructor_wrong_argument_type` |
| `new!(Dog("Rex", 3))`，参数个数不对 | E0061（同时指向构造器） | remove the extra argument | `constructor_wrong_argument_count` |
| `Dog::new("Rex".into())` | E0283 type annotations needed | 去掉 `.into()` | `string_into_breaks_inference` |
| `#[roost::main(stak = ..)]` | unknown argument `stak` | did you mean `stack`? | `bad_args` |
| `stack = "64 parsecs"` | invalid size | 写 `"64MB"` 这样的大小 | `bad_size` |
| `fn main(args: Vec<String>)` | `fn main` takes no parameters | 用 `std::env::args()` | `main_with_args` |
| `#[roost::main]` 放在别的函数上 | goes on the program's `fn main` | 改名为 `main` 或去掉属性 | `not_main` |
| `fn main() -> i32` | needs `fn main()` or `fn main() -> Result<(), E>` | 退出码用 `std::process::exit` | `wrong_return` |

还有两个常见的，报错来自 rustc 本身，没有专门的提示：`a == b` 比较两个 `Ref`（E0369，改用
`Ref::ptr_eq(&a, &b)` 或 `a.equals(b.clone())`），以及类外读字段 `dog.name`（E0609，改用 getter）。

方法体里的普通 Rust 错误（类型不匹配、拼错方法名、漏写语句末尾的 `;`……）由 rustc 报告，位置同样在你的代码上。
一个方法体里有 `class!` 自己能发现的错误时，宏会先只报这个错，同一个方法体里的其他错误要等它修好才会出现。

## 编辑器与格式化（rustfmt）

**rustfmt 不会格式化 `class! { .. }` 里面的代码**（它不格式化任何花括号宏的内容），也不会改动它，所以开着
「保存时格式化」是安全的：宏外面照常格式化，宏里面保持你写的样子。建议：

- **手动保持 Java 风格的排版**：4 空格缩进，每个成员一行，花括号不换行（和本仓库的例子一样）。可以在项目里放一个
  `.editorconfig` 让编辑器帮你：

  ```ini
  [*.rs]
  indent_style = space
  indent_size = 4
  max_line_length = 100
  ```

- **VS Code + rust-analyzer**：保持 `rust-analyzer.procMacro.enable` 为默认的 `true`，rust-analyzer 会展开 `class!`，
  `cargo check` / `clippy` 的报错（包括上面那些带 `help:` 的）会标在编辑器里对应的位置。`.vscode/settings.json`：

  ```json
  {
    "[rust]": { "editor.formatOnSave": true, "editor.tabSize": 4 },
    "rust-analyzer.procMacro.enable": true,
    "rust-analyzer.check.command": "clippy"
  }
  ```

  宏里面的补全、跳转依赖 rust-analyzer 对 nightly proc macro 的支持，我们没有逐项验证；不灵的时候以
  `cargo check` 的结果为准。
- **IntelliJ / RustRover**：打开 proc macro 展开（设置项的位置随版本不同），格式化器选 rustfmt。
- 想让一段较长的逻辑也被 rustfmt 格式化：把它写成类外的普通函数，在方法里调用。比如
  `public fn report() -> String { report_of(this.name, this.age) }`，`fn report_of(..)` 写在 `class!` 外面。

## 文档

- [教程](docs/tutorial.md)：以一个小动物园为例，覆盖全部特性。
- [设计说明](docs/design.md)：运行时、`class!` 生成了什么、语义细节、编译期诊断的做法、第一棒 spike 的记录。
- API 文档：`cargo doc -p roost --open`。
- 示例：`cargo run -p roost --example zoo`（类、继承、`#[roost::main]`、`new!`）、
  `cargo run -p roost --example shapes`（接口、抽象类、静态成员、`toString` / `equals`）。

## 开发

```sh
cargo test --workspace                  # 单元测试、class! 端到端测试、trybuild、doctest（含 README 和教程）
cargo clippy --workspace --all-targets -- -D warnings
cargo miri test -p roost --lib          # 运行时的 unsafe 代码（CI 还跑 --test class / oop）
TRYBUILD=overwrite cargo test -p roost --test compile_fail        # 重新生成 .stderr 快照（审阅 diff！）
TRYBUILD=overwrite cargo test -p roost-main-tests --test main     # 同上，#[roost::main] 的
```

`.stderr` 快照绑定在钉住的 nightly 上，升级工具链后要重新生成并审阅 diff。CI 的全部步骤见
[`.github/workflows/ci.yml`](.github/workflows/ci.yml)。
