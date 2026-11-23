# roost

An experiment: a Java-like object model for Rust — classes, field inheritance, overridden
methods dispatched through a vtable, and instanceof-style downcasting — on top of a tracing
garbage collector, all behind macros.

Status: **class DSL**. `roost::class!` declares classes with fields, single
inheritance of any depth, constructors, virtual methods, `super`, and `this.field`; interfaces
(with `extends` and `default` methods), `abstract` / `final` classes and methods, `static`
fields (thread-local) and methods, and `toString` / `equals` / `hashCode` overrides. The
runtime underneath is tested on its own. The first spike lives on as a regression suite.

```sh
cargo test                              # runtime unit tests + class! tests + trybuild + spike + doctests
cargo run -p roost --example zoo        # classes, extends, super, cast
cargo run -p roost --example shapes     # interface + abstract + static + Object methods
cargo run -p roost --example spike      # walks through the spike's five demos
cargo miri test -p roost --lib          # the runtime's unsafe code under Miri (CI also runs --test class / oop)
TRYBUILD=overwrite cargo test -p roost --test compile_fail   # regenerate .stderr snapshots (review the diff!)
```

```rust
use roost::prelude::*;

class! {
    public class Animal {
        protected name: String;
        private legs: i32 = 4;

        public Animal(name: String) {
            this.name = name;
            this.init();              // 虚调用，落到子类
        }

        public fn speak() -> String { format!("{} makes a sound", this.name) }
        protected fn init() {}
    }

    public class Dog extends Animal {
        private tricks: i32;

        public Dog(name: String) { super(name); }

        @Override
        public fn speak() -> String { super.speak() + ", woof" }

        @Override
        protected fn init() { this.tricks = 1; }
    }
}

let a: Ref<Animal> = Dog::new("Rex".into());   // 隐式向上转型（不需要 .into()）
a.speak();
a.instanceof::<Dog>();
let d: Option<Ref<Dog>> = a.cast::<Dog>();
```

## 工具链

- 钉在 **`nightly-2026-11-23`**（`rust-toolchain.toml`），fmt / clippy / test / Miri 都在它上面验证过。
  不再被 gc 卡住，可以随时往后升：升完跑一遍 CI 的全部步骤即可。
- gc 用的是 **vendor 进来并打过补丁的 gc 0.5.1 / gc_derive 0.5.0**（`vendor/`，说明见
  [`vendor/README.md`](vendor/README.md)）。原版 gc 0.5.1 的 `nightly` feature 从 nightly-2026-08-27 起
  编不过（`GcBox::from_box` 里 E0606），上游 master 也没修；补丁是一行类型标注。另外把 gc_derive 升到
  synstructure 0.13，`#[derive(Trace)]` 不再报 `non_local_definitions`。哪些补丁能回馈上游见
  [`UPSTREAM.md`](UPSTREAM.md)。
- `Gc<dyn Trait>` 只在 gc 的 `nightly` feature 下存在（`CoerceUnsized` / `DispatchFromDyn` / `Unsize`）。
  roost 自己开了 `coerce_unsized`、`dispatch_from_dyn`、`unsize`（给 `Ref<T>` 同样的能力）和
  `set_ptr_value`（`Ref::cast` 里重建胖指针）。
- CI 会检查 `rustc -V` 确实是 nightly。

## 结构

cargo workspace，`vendor/` 不是 member（不让我们的 `clippy -D warnings` 去查上游代码），CI 单独跑它的测试。

| 路径 | 内容 |
| --- | --- |
| `roost/` | 运行时，见下 |
| `roost-macros/` | proc-macro `class!`：`model.rs`（语法树 + 解析）、`index.rs`（继承链、接口闭包、每个方法用哪个实现、`@Override` / abstract / final 等检查）、`rewrite.rs`（改写 `this.field` / `Class::static_field` / `super`）、`expand.rs`（生成代码）。诊断信息有单元测试 |
| `roost/tests/class.rs` | `class!` 的端到端测试（三层继承 + 一个兄弟类） |
| `roost/tests/oop.rs` | interface / abstract / final / static / Object 方法的端到端测试 |
| `roost/tests/compile_fail.rs`，`roost/tests/compile_fail/` | trybuild：每个编译期报错一个 `.rs` + `.stderr` 快照 |
| `roost/examples/zoo.rs` | `class!` 的演示 |
| `roost/examples/shapes.rs` | interface + abstract + static + `toString` / `equals` 的演示 |
| `roost/tests/spike.rs` | 第一棒 spike 的测试（五件事 + GcCell 冲突的证据），原样保留 |
| `roost/tests/spike/` | spike 的对象模型：`object.rs`（`base_class!` / `derived_class!`）、`zoo.rs`、`probe.rs` |
| `roost/examples/spike.rs` | spike 的演示，和测试共用 `tests/spike/` 下的模块 |
| `vendor/gc`，`vendor/gc_derive` | 打过补丁的 gc |

### `roost` 运行时

后面 `class!` 生成的代码就建在这几样东西上。都有单元测试（`roost/src/*.rs` 里的 `mod tests`，用的类层次
`roost/src/test_classes.rs` 是手写的「宏大概会生成成这样」：`Animal` → `Dog`（非叶子）→ `Puppy`，以及 `Cat`）。

- **`Ref<T: ?Sized>`**：包一个 `Gc<T>`。`Clone`（复制引用，不复制对象）、`Deref`；`Ref<Puppy>` 能隐式转成
  `Ref<dyn Dog>` / `Ref<dyn Animal>` / `Ref<Object>`（`CoerceUnsized`，trait 之间靠 trait upcasting）；
  `DispatchFromDyn`，所以类 trait 里可以写 `self: Ref<Self>` 的方法。`Ref::ptr_eq` 是 Java 的引用 `==`，
  两边静态类型可以不同，只比地址不比 vtable。故意没有 `PartialEq`。`Display` / `Debug` 走 `ObjectClass::to_string`。
- **`ObjectClass`**：根类 trait；**`Object`** 是 `type Object = dyn ObjectClass`，和类一样写 `Ref<Object>`。
  `to_string`（默认 `类名@十六进制hash`）、`hash_code`（按地址的 identity hash，gc 不移动对象所以稳定）、
  `equals(other: Ref<Object>)`（默认 identity）、`class_name`，都有默认实现；前三个的默认实现另有自由函数版本
  （`super.to_string()` 落到它们）。`Any` 支持：`AsAny` 是 `ObjectClass` 的父 trait，`animal.as_any()` 经过
  vtable 拿到具体类的 `&dyn Any`。
- **`instance_of` / `cast`**：`r.instance_of::<dyn Dog>()`，`r.cast::<dyn Dog>() -> Option<Ref<dyn Dog>>`，
  失败返回 `None`。目标可以是具体类，也可以是**非叶子类的 trait object**（`Ref<dyn Animal>` → `Ref<dyn Dog>`，
  spike 做不到这个）。做法：`cast` 虚调用 `ObjectClass::__roost_casts`，具体类在里面把自己作为它能被看成的每个类型
  「报上来」（`req.offer_class(self).offer::<dyn Dog, _>(self)...`，以后由宏生成），只有类型匹配的那一项被采用。
  这个方法是 safe 的，写错了也不会 UB：`cast` 会核对报上来的指针确实来自这个对象本身（同一地址、同一个运行期
  类型），否则当作失败（有测试故意「撒谎」）。
- **`roost::gc`**：`collect()`（立刻全量回收），gc 类型的 re-export，以及测试辅助 `Counter` / `Probe`：
  把 `counter.probe()` 放进对象的字段，`counter.finalized()` / `counter.dropped()` 就是这些对象被 finalize /
  被释放的次数。不是线程本地全局量，每个测试自己 new 一个。

另外两样只给生成代码用：`Ref::upcast`（表达式里的显式向上转型），`roost::__private::{get, set, get_static,
set_static}`（字段 / 静态字段读写，各自只在函数内部短借用）。`roost::prelude` 导出 `Ref`、`Object`、
`ObjectClass`、`class!`。

`#[derive(Trace)]` 展开成 `::gc::Trace`、用户 crate 必须直接依赖 `gc` 的问题，对 `class!` 不存在：它自己生成
`Trace` impl，路径都经由 `::roost`。`ObjectClass::__roost_casts` 也由宏生成，祖先和接口列表不会漏。

## `class!` DSL

### 语法

```text
class     := attr* ("public")? ("abstract" | "final")? "class" Name ("extends" Parent)?
             ("implements" Iface ("," Iface)*)? "{" member* "}"
interface := attr* ("public")? "interface" Name ("extends" Iface ("," Iface)*)? "{" imember* "}"
member    := attr* ("@Override")? access modifier* ( field | ctor | method )
modifier  := "static" | "abstract" | "final"                     // 顺序随意
field     := name ":" Type ("=" expr)? ";"                       // static 字段同样写法
ctor      := Name "(" params ")" { ("super(" args ");")? stmts } // Name 就是类名
method    := "fn" name "(" params ")" ("->" Type)? (block | ";") // ";" 当且仅当 abstract
imember   := attr* ("public")? ( "fn" sig ";" | "default" "fn" sig block | "static" "fn" sig block )
access    := "public" | "protected" | "private" | 空（包级）
```

方法体里用 `this`，不写 `self` 参数。`static fn` 是没有 `this` 的关联函数，调用写 `Animal::count()`；
`static` 字段在类里的任何方法 / 构造器里写 `Animal::count` 或 `Self::count`（读、`=`、`+=` 都行）。

```rust
class! {
    public interface Named { fn name() -> String; }
    public interface Pet extends Named {
        fn owner() -> String;
        default fn greet() -> String { format!("{} greets {}", this.name(), this.owner()) }
    }

    public abstract class Animal implements Named {
        protected name: String;
        private static created: i32 = 0;
        protected Animal(name: String) { this.name = name; Animal::created += 1; }
        @Override public fn name() -> String { this.name }
        public abstract fn legs() -> i32;
        public final fn kind() -> String { "animal".into() }
        public static fn created() -> i32 { Self::created }
    }

    public final class Dog extends Animal implements Pet {
        public Dog(name: String) { super(name); }
        @Override public fn legs() -> i32 { 4 }
        @Override public fn owner() -> String { "Ann".into() }
        @Override public fn to_string() -> String { format!("Dog({})", this.name) }
    }
}

let p: Ref<Pet> = Dog::new("Rex".into());
p.greet();                         // "Rex greets Ann"
p.instanceof::<Named>();           // true
let a: Ref<Animal> = p.cast::<Animal>().unwrap();
format!("{a}");                    // "Dog(Rex)"：Display 走 toString
Animal::created();                 // 1
```

### 生成了什么

以 `class Dog extends Animal` 为例：

| 生成物 | 作用 |
| --- | --- |
| `type Dog = dyn DogClass;` | 用户写的 `Ref<Dog>` 就是多态引用，不出现 `dyn`。edition 2024 禁止裸 trait object，用别名绕开 |
| `trait DogClass: AnimalClass + PetInterface` | vtable：Dog **新引入**的虚方法 + Dog 每个字段一个隐藏访问器 `__roost_field_Dog_tricks(&self) -> &GcCell<i32>`。父 trait 是父类的 trait（根类是 `roost::ObjectClass`）加上 `implements` 的每个接口的 trait |
| `type Pet = dyn PetInterface;`、`trait PetInterface: ObjectClass + NamedInterface` | 接口：方法声明（含 `default` 方法）。`default` 方法体和 `static` 方法在 `impl dyn PetInterface` 里，和类的方法体一样对 `T` 泛型 |
| `thread_local! { static __ROOST_STATIC_Animal_created: GcCell<i32> }` | static 字段，每个线程一份（gc 也是线程本地的），第一次访问时求初值，像 Java 的类初始化 |
| `struct __RoostDog` | 运行期类**恰好是** Dog 的对象（抽象类不生成）。继承链上所有类的字段**平铺**在里面（根类在前，每个字段一个 `GcCell`，名字带类名，所以父子类字段同名也不冲突）。`Trace` / `Finalize` / `ObjectClass` 都是生成的，不依赖 `#[derive(Trace)]`（所以用户 crate 不用直接依赖 `gc`） |
| `impl AnimalClass for __RoostDog`、`impl DogClass for __RoostDog`、`impl PetInterface for __RoostDog`、… | 链上每个类、实现的每个接口（含父接口、父类实现的接口）一个 impl。每个方法转发到**编译期解析好**的实现：最近的类里的方法体，没有就是接口的 `default`，所以三层、N 层都一样。`impl ObjectClass` 里转发覆盖了的 `to_string` / `equals` / `hash_code` |
| `impl dyn DogClass { fn new(..), .. }` | `Dog::new(..)` 构造器；方法体都在这里，形如 `fn __roost_method_speak<T: ?Sized + DogClass>(this: &T)`。抽象类的 `new` 带一个永远不满足的约束（见下） |
| `trait __RoostBook_title` | 只在「同一个方法被有关系的两个类型都声明」时出现：类 `Book` 有 `title()`，`Novel extends Book implements Titled` 而接口 `Titled` 也声明了 `title()`。若两个 trait 各自声明，`novel.title()` 会 E0034 歧义，而 trait object 上的歧义**没法**用 inherent 方法盖掉（试过）。所以把这个方法单独放进一个 trait，`BookClass` 和 `TitledInterface` 都以它为父 trait：沿两条路径到达的是同一个方法。几处声明的签名必须一致，否则宏报错 |

关键在最后一条：**方法体和构造器体都对 `this` 的具体类 `T` 泛型**，按具体类单态化。所以 Animal 的构造器在构造
Dog 时是以 `T = __RoostDog` 实例化的，里面的 `this.init()` 静态地落到 Dog 的覆盖版本，这就是 Java 语义
（spike 是 C++ 语义）。`super.speak()` 是对父类实现的直接调用 `<dyn AnimalClass>::__roost_method_speak(this)`。
`Ref<Animal>` 上的 `a.speak()` 才走 vtable。

为什么能在编译期解析：一个 `class!` 块里的所有类宏都看得到，每个类的继承链、每个方法由谁实现、每个
`this.x` 是哪个类的字段，展开时就定下来了。代价见下面「限制」第一条。

### 语义

- **构造顺序（Java）**：`Dog::new(..)` 先分配完整对象，所有字段取默认值（`= expr`，否则 `Default::default()`，
  根类的字段先求值），然后从根类到子类依次执行构造器体。`super(..)` 只能是构造器第一条语句；不写就隐式
  `super()`，父类没有无参构造器时报错。构造器里的虚调用落到子类，子类字段此时是默认值（和 Java 一样）。
  测试：`virtual_call_in_a_constructor_lands_in_the_subclass`。
- **字段访问**：`this.f` 读 → `get(cell)`，在一次短借用里 **clone** 出来；`this.f = e` → `set(cell, e)`；
  `this.f op= e` → 读出、算右边、写回（Java 的求值顺序：右边的副作用对这个字段的写会被覆盖，有测试）。
  guard 从不离开这两个辅助函数，所以「基类拿着 borrow_mut 调虚方法，子类覆盖去读同一字段」的 panic
  **在结构上不可能发生**（`base_method_calling_an_override_that_touches_the_same_field_does_not_panic`）。
  格式化宏（`format!(.., this.name)`）里的字段读也会改写。
- **方法默认是虚方法**（public / protected / 包级），`private` 方法不是虚方法（不进 vtable，子类同名方法是新方法），
  `static` 方法也不是。
- `@Override`：标了但父类链、实现的接口、`Object` 里都没有这个方法 → 编译错误；覆盖父类方法、实现接口方法、
  覆盖 `Object` 方法却没标 → **编译错误**（Java 只是
  可选注解，这里更严：proc macro 在 stable API 上发不了 warning）。参数 / 返回类型和父类不一致时 rustc 报
  E0053，同时指向覆盖处和父类声明处。
- `instanceof` / `cast` 到任意祖先（包括非叶子类）**和任意实现了的接口**（`instanceof::<Pet>()`、
  `cast::<Pet>()`，接口之间、接口回到类也行）都由宏生成 `__roost_casts`，失败返回 `None`。`Ref::ptr_eq`
  是引用相等。`Ref<Dog>` → `Ref<Pet>` → `Ref<Named>` → `Ref<Object>` 都是隐式转换。
- **接口**：方法隐式 public；没有方法体的是抽象方法，`default fn` 带方法体，`static fn` 是静态方法
  （`Pet::species()`）。`interface A extends B, C`。类实现接口方法要写 `@Override`（同覆盖，见上）。
  方法由谁实现按 Java 的规则：继承链上最近的类（父类里继承来的方法也能实现接口方法）优先于接口的 `default`；
  父类里的 `abstract` 声明优先于接口的 `default`（子类仍须实现）。接口不能有字段、构造器、private 方法。
- **abstract**：`abstract class` 不能 new：`Shape::new(..)` 在调用处报 `E0277 cannot instantiate abstract
  class `Shape``（靠 `#[diagnostic::on_unimplemented]` 和一个永远不满足的约束）。抽象类可以有构造器（给子类
  `super(..)` 用）。`abstract fn` 只能出现在 abstract class 里、不能有方法体、不能是 `private` / `static` /
  `final`。具体类没实现所有抽象方法（来自抽象父类或接口）时报错，**报错位置是这个具体类的类名**，列出缺的方法。
- **final**：`extends` 一个 `final class` 报错（指向 `extends` 后面的父类名）；覆盖 `final fn` 报错（指向子类的方法名）。
- **static**：`static` 字段放在线程本地存储里（`thread_local!` + `GcCell`），存 `Ref` 也没问题（它是 GC root）。
  每个线程有自己的一份，和 gc 的堆一样。`private static` 只有声明它的类能访问。类里也可以写 `Dog::created()`
  调父类的静态方法（宏改写成 `Animal` 的）。静态字段和静态方法可以同名（`Animal::count` 和 `Animal::count()`，
  和 Java 一样分开）。
- **`Object` 的方法**：`to_string`、`equals`、`hash_code` 可以用 `@Override` 覆盖（不标也是错误），
  `super.to_string()` 等可以调到父类的版本或 `Object` 的默认实现。`Ref` 的 `Display`（`format!("{r}")`、
  `r.to_string()`）和 `Debug` 走 `to_string()`。签名：`to_string() -> String`、`hash_code() -> i32`、
  `equals(other: Ref<Object>) -> bool`，实现里用 `other.cast::<Point>()` 判断类型。
- `Ref<X>`、`Option<Ref<X>>`（以及任何 `Trace` 类型）的字段都会被追踪，对象环能被回收
  （`cycle_across_three_levels_is_collected`，CI 里同样验证了「跳过 collect 时这个测试会失败」）。

### 和 Java 的差异

| 方面 | Java | roost |
| --- | --- | --- |
| 向上转型 | 隐式 | `let a: Ref<Animal> = dog;` 隐式；表达式中间用 `dog.upcast::<Animal>()`。**`.into()` 做不到**：`impl From<Ref<T>> for Ref<U> where T: Unsize<U>` 和 core 的 `impl<T> From<T> for T` 冲突（E0119），所以示例里的 `.into()` 去掉了 |
| 读字段 | 读到值 / 引用 | 读到一份 **clone**。对 `i32`、`String`、`Ref<X>` 这和 Java 一样；对 `Vec` 这类值类型，`this.items.push(x)` 改的是副本——所以宏把方法调用的接收者做成 `&副本`，`&mut self` 方法**编译不过**（E0596），而不是静默丢失修改。要改就读出来、改、写回。`this.p.x = 1`、`this.v[0] += 1`、`&mut this.f` 也直接报错。字段类型必须 `Clone`（读的时候）和 `Trace` |
| 字段默认值 | `null` / 0 / false | `Default::default()`。没有 null：可空引用写 `Option<Ref<X>>`；`Ref<X>` 本身不能做没有初始化器的字段 |
| 字段初始化器 | 在各自构造器里、`super()` 之后执行，可以用 `this` | 分配时一次性求值（根类在前），不能用 `this` |
| 访问别的对象的字段 | `other.name` | 只支持 `this.f`。字段不能从类外访问（`public` 字段也不行），请写 getter |
| `public` / `protected` / 包级 / `private` | | 类、构造器、static 方法：`pub` / `pub(crate)` / `pub(crate)` / 私有（模块内）。**虚方法都在 trait 里，Rust 的 trait 方法没有单独的可见性**，所以 protected / 包级虚方法实际和类一样可见。`private` 字段：子类里的 `this.x` 在宏展开时报错；`private` 方法生成为模块私有函数 |
| 重载 | 支持 | 不支持（每个类最多一个构造器，方法不能同名） |
| 字段隐藏 | 子类同名字段隐藏父类的 | 一样（存储按类名区分） |
| `this` 作为值 | 可以存起来、传出去 | 方法体里 `this` 是 `&T`，不能存成 `Ref`。环只能从外部建（`a.befriend(b)`） |
| 泛型方法 / 泛型类 | 支持 | 不支持（泛型方法不能进 vtable；泛型类见「限制」） |
| 静态方法调用 | 可以省略类名 | 要写 `Animal::f()`（在本类方法体里也可以写 `Self::f()`；`Dog::f()` 调继承来的静态方法只在类里有效，类外要写声明它的类 `Animal::f()`） |
| 静态字段 | `Animal.count`，可省略类名；全进程一份 | 写 `Animal::count` / `Self::count`，**只能在类里**用（类外请写 static getter，和实例字段一样）。**每个线程一份**（`thread_local!`），因为 gc 堆是线程本地的；初值在该线程第一次访问时求值 |
| `==` | 引用比较 | **不重载 `==`**：`Ref` 没有 `PartialEq`。引用相等用 `Ref::ptr_eq(&a, &b)`，值相等用 `a.equals(b.clone())`。理由：Java 的 `==` 对引用是同一性，Rust 读者看到 `==` 会以为是值比较；两种都写成显式的名字就不会混 |
| `Object` 的方法名 | `toString()` / `equals(Object)` / `hashCode()` | Rust 命名：`to_string()` / `equals(other: Ref<Object>)` / `hash_code()`，用 `@Override` 覆盖。`equals` 的参数按值收 `Ref<Object>`（任何 `Ref` 都能隐式转过去），调用时通常要 `b.clone()`（复制引用，不复制对象）。`class_name()`（≈ `getClass().getSimpleName()`）不能覆盖 |
| `Object` 类型 | `Object o` | `Ref<Object>`（`type Object = dyn ObjectClass`）。不能写 `extends Object`，也不需要 |
| 接口 | 可有常量、`private` 方法、`static` 方法、泛型 | 只有抽象方法、`default` 方法、`static` 方法。子接口不能重新声明 / 覆盖父接口的方法（包括 `default`）；`X.super.m()` 不支持 |
| 实现接口方法 | `@Override` 可选 | `@Override` **必须**写（和覆盖一样，见上） |
| 同名方法冲突 | 两个接口的同名方法由一个实现满足 | 一样（见上面 `__RoostBook_title`）。但几处声明签名必须完全一致（Java 允许返回类型协变，这里不行） |
| `abstract` 方法覆盖 | 抽象方法可以「重新抽象」一个继承来的具体方法 | 不支持：`abstract fn` 不能覆盖 / 重新声明任何方法 |
| `final` | 类、方法、字段、局部变量 | 只支持 `final class` 和 `final fn`。`final` 字段（含 `static final` 常量）不支持，会报错；常量请用 Rust 的 `const` |

### 限制

- **一个继承树必须写在同一个 `class!` 块里**。`extends` 不在当前块里的类会报错（错误信息说明了这一点）。
  跨块 / 跨模块 / 跨 crate 继承需要把父类的元信息传给子类的展开（比如父类展开时顺带生成一个
  `macro_rules!` 携带自己的声明），留给后面。
- **不支持泛型类**（`class Box<T: Trace> { value: T; }`，任务里的可选项，评估后没做）。`type Box<T> = dyn BoxClass<T>`
  本身写得出来，难的是其余部分全要带上类型参数：字段的扁平存储 `__RoostBox<T>`、`Trace` impl 的约束、
  `extends Box<String>` 时把父类的字段类型 / 方法签名里的 `T` 代换掉（index 现在按名字解析，不做类型代换）、
  `cast` 需要 `T: 'static` 且不同实例化是不同的类、方法体已经对 `this` 的具体类泛型，还要再叠一层。代价接近
  再写一遍 index，收益在 Rust 里有限（泛型容器本来就有）。现在写 `class Box<T>` 报 `generic classes are not supported`。
- 用户 crate 要用 nightly 工具链，但**不需要**写任何 `#![feature]`：unsize 相关的 impl 都在 roost 里。
- `rustfmt` 不格式化 `class! { }` 里面。
- 诊断：方法体里的错误、覆盖签名不一致（E0053）、字段类型不是 `Trace`、拼错方法名，都指向用户代码；
  宏自己的检查（`@Override`、private、`super(..)` 位置、未知父类、abstract / final、缺实现……）也指向用户写的 token。
  这些编译期报错在 `roost/tests/compile_fail/` 里都有 trybuild 用例和 `.stderr` 快照（快照绑定在钉住的 nightly 上，
  升级工具链后用 `TRYBUILD=overwrite` 重新生成并审阅 diff）。
- 某个方法体里有宏能发现的错误（例如静态方法里用 `this`、未知字段）时，宏报错后把整个方法体换成
  `unreachable!()`，避免 rustc 在占位表达式上堆出无关的后续错误；所以同一个方法体里的其他错误要等这个修了才会出现。
- `new` 抽象类的报错还带着生成代码的痕迹（`the trait __RoostAbstractShape is not implemented for &()`
  这类 help 行），主消息和位置是对的。

## 第一棒 spike（PR #1）

下面是第一棒的记录，代码路径已更新到新位置。当时 gc 还没 vendor，toolchain 钉在 nightly-2026-08-20。

| 文件 | 内容 |
| --- | --- |
| `roost/tests/spike/object.rs` | `Object` / `Class` trait、`GcExt::{is, cast}`，以及 `base_class!` / `derived_class!` 两个宏 |
| `roost/tests/spike/zoo.rs` | 用宏写出来的 `Animal`（根类）→ `Dog` |
| `roost/tests/spike/probe.rs` | 线程本地的 drop / finalize 计数器 |
| `roost/tests/spike.rs` | 五件事的断言 + GcCell 冲突的证据 |
| `roost/examples/spike.rs` | `cargo run -p roost --example spike` 演示 |

`base_class!` 对 `Animal` 生成：

- `struct AnimalBase { name, friend, first_words, probe }`：根类的字段，本身也可实例化；
- `trait Animal: Object`：vtable。引用类型是 `Gc<dyn Animal>`。每个虚方法都有默认实现，转发到基类实现；
- `mod animal_super`：每个虚方法对应一个 `fn speak<T: ?Sized + Animal>(this: &T)`，也就是基类那一份，
  子类里的 `super.speak()` 写成 `animal_super::speak(this)`；
- `fn animal(&self) -> &AnimalBase`（通过 vtable 访问继承来的字段）和
  `fn as_animal(self: Gc<Self>) -> Gc<dyn Animal>`（把 `this` 变成 GC 指针）。

`derived_class!` 生成 `struct Dog { pub base: AnimalBase, breed }`（字段包含式继承），并且
`impl Animal for Dog`：被覆盖的方法用用户写的方法体，没覆盖的沿用默认实现。

### 五件事，以及证据在哪

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
ROOST_SKIP_COLLECT=1 cargo test -p roost --test spike five_   # 两个环测试都失败
```

CI 里有一步专门跑这条命令，**要求它失败**。直接把 `collect();` 那一行删掉也验证过，同样失败
（`both objects finalized: left 0, right 2`）。

### 实践下来真正难的地方

#### 1. 字段可变性：GcCell 的 borrow 冲突多容易撞上？撞上 panic 能接受吗？

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
（`roost/tests/spike/zoo.rs` 里 `Dog::on_renamed`），而不是一直拿着 `borrow_mut` 的那一帧。要找到真正的「持有者」得开
`RUST_BACKTRACE=1`，在栈里往上翻到 `animal_super::rename_holding_borrow::<Dog>`。

能不能接受：作为 spike，能定位、能修（把 guard 的作用域缩到调用之前，见 `rename`）。作为「像 Java
一样写类」的体验，不太能接受：Java 用户默认字段随便读写，这里冲突是运行期的，还依赖动态类型。
下一棒建议让宏生成 `get_name() -> String` / `set_name(v)` 这类**不把 guard 交给用户**的访问器，
使借用永远不跨越方法调用；需要原地改大字段的场景再单独开口子。

#### 2. `this` 的类型：最后写成什么样？`&self` 和 `this: &Gc<dyn X>` 哪个顺手？

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

#### 3. super 调用和「构造器里调用被子类覆盖的方法」：宏藏住了多少，还剩哪些陷阱？

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

#### 4. 报错可读性：宏生成的代码出错时指向哪里，能用吗？

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

#### 其他实际踩到的

- **gc 0.5.1 + nightly 已经在腐烂。** 它的 `nightly` feature 在 2026-08-27 之后的 nightly 上编译不过。
  当时只能钉在旧 nightly 上。（已解决：vendor + 补丁，见上文工具链。）
- `gc_derive 0.5.0` 的 `#[derive(Trace)]` 在新编译器上每用一次都报 `non_local_definitions` 警告，
  只能在 crate 级 `allow`。（已解决：vendor 的 gc_derive 升到了 synstructure 0.13。）
- 宏写 `Gc<dyn Trait>` 的默认方法时 `Self: ?Sized` 处处碍事（不能 unsize、不能 `&Self as &dyn Trait`），
  只能给每个类生成小的转换方法（`animal()`、`as_animal()`）放进 vtable。

## 第一棒留下的建议

1. ~~先决定 gc 依赖怎么办~~：已 vendor 并修好，见 `vendor/README.md`、`UPSTREAM.md`。
2. ~~换 proc macro~~：`class!`，任意层继承，见上文 DSL。
3. ~~字段访问由宏生成 getter / setter~~：`this.field` 改写成短借用的 `get` / `set`。
4. ~~两段式构造~~：比两段式更进一步，构造器体对具体类泛型，构造期虚调用就是 Java 语义。
5. ~~支持 cast 到非叶子类（`Gc<dyn Dog>`）~~：`Ref::cast::<dyn Dog>()`，见上文运行时。
