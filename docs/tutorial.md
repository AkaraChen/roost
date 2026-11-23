# roost 教程：用 Java 的写法搭一个动物园

这篇教程面向**会 Java、刚开始写 Rust** 的读者。我们从一个只有一个类的程序开始，一步步加上继承、接口、
抽象类、静态成员、`toString` / `equals`，最后得到一个完整的小动物园。每一节的代码都是完整的，可以直接
粘贴运行；它们也是 roost 测试的一部分（`cargo test --doc` 会把本文里的每个 `rust` 代码块编译一遍）。

目录：

1. [准备](#1-准备)
2. [第一个类](#2-第一个类)
3. [字段、访问修饰符和「读到的是副本」](#3-字段访问修饰符和读到的是副本)
4. [继承、`super` 和 `@Override`](#4-继承super-和-override)
5. [多态：`Ref`、`instanceof`、强制转换](#5-多态refinstanceof强制转换)
6. [接口](#6-接口)
7. [抽象类和 `final`](#7-抽象类和-final)
8. [静态字段和静态方法](#8-静态字段和静态方法)
9. [`toString`、`equals`、`hashCode`](#9-tostringequalshashcode)
10. [`null` 去哪了：可空引用和对象环](#10-null-去哪了可空引用和对象环)
11. [程序入口 `#[roost::main]`](#11-程序入口-roostmain)
12. [编译器报错时](#12-编译器报错时)
13. [完整的动物园](#13-完整的动物园)

---

## 1. 准备

roost 需要 **nightly** 工具链（它用到 Rust 还没稳定的 trait object 转换）。在你的项目根目录放一个
`rust-toolchain.toml`，内容和 roost 仓库里的一样：

```toml
[toolchain]
channel = "nightly-2026-11-23"
components = ["rustfmt", "clippy"]
```

`Cargo.toml`：

```toml
[dependencies]
roost = { git = "https://github.com/AkaraChen/roost" }
```

你自己的代码**不需要**写任何 `#![feature(..)]`。每个文件开头加一行：

```rust
use roost::prelude::*; // class!、new!、Ref、Object
```

## 2. 第一个类

Java：

```java
public class Animal {
    private String name;

    public Animal(String name) { this.name = name; }

    public String speak() { return name + " makes a sound"; }
}

public class Main {
    public static void main(String[] args) {
        Animal a = new Animal("Rex");
        System.out.println(a.speak());
    }
}
```

roost：

```rust
use roost::prelude::*;

class! {
    public class Animal {
        private name: String;

        public Animal(name: String) { this.name = name; }

        public fn speak() -> String { format!("{} makes a sound", this.name) }
    }
}

#[roost::main]
fn main() {
    let a = new!(Animal("Rex"));
    println!("{}", a.speak());
}
```

对照着看，变化只有几处：

| Java | roost | 说明 |
| --- | --- | --- |
| `String name;` | `name: String;` | Rust 的写法：名字在前，类型在后 |
| `String speak()` | `fn speak() -> String` | 方法以 `fn` 开头，返回类型写在 `->` 后面；没有返回值就不写 `->`（Java 的 `void`） |
| `return x;` | `x`（最后一个表达式） | `return` 也能用，但 Rust 习惯让最后一个表达式作为返回值 |
| `name + "..."` | `format!("{} ...", this.name)` | 字符串拼接用 `format!` |
| `new Animal("Rex")` | `new!(Animal("Rex"))` | 也可以写成 `Animal::new("Rex")`，两者完全一样 |
| `public static void main(String[] args)` | `#[roost::main] fn main()` | 命令行参数用 `std::env::args()` |

几件值得注意的事：

- **方法体里用 `this`**，和 Java 一样，而且 `this.` 不能省略（`this.name`，不能只写 `name`）。不要写 Rust 的 `self`。
- 构造器的名字就是类名，和 Java 一样。
- `String` 类型的构造器参数可以直接传字符串字面量 `"Rex"`，也可以传 `String`、`&String`、`format!(..)` 的结果。
  **不要**写 `"Rex".into()` 或 `"Rex".to_string()`：前者会让编译器推断不出类型（见[第 12 节](#12-编译器报错时)），
  后者能用但多余。
- `#[roost::main]` 不是必需的（普通的 `fn main` 也能用 `class!`），但它会给程序一个大栈、在退出前回收一次
  垃圾，并把 roost 的运行时异常打印成 Java 的样子。第 11 节细说。

`a` 的类型是 `Ref<Animal>`：**一个指向堆上 `Animal` 对象的引用**，相当于 Java 里 `Animal a` 这个变量。
`a.clone()` 复制的是引用，不是对象；对象由垃圾回收器管理，不用手动释放。

## 3. 字段、访问修饰符和「读到的是副本」

```rust
use roost::prelude::*;

class! {
    public class Animal {
        protected name: String;      // 子类可以访问
        private legs: i32 = 4;       // 带初始值
        private meals: Vec<String>;  // 没写初始值：取默认值，空 Vec
        age: i32;                    // 不写修饰符：包级（crate 内可见）

        public Animal(name: String) { this.name = name; }

        public fn eat(food: String) {
            // 读字段得到的是一份副本，改副本不会改到字段：
            // 先读出来，改，再写回去。
            let mut meals = this.meals;
            meals.push(food);
            this.meals = meals;
        }

        public fn meals() -> usize { this.meals.len() }   // 只读的方法调用可以直接写

        public fn birthday() { this.age += 1; }            // += 这类复合赋值直接支持

        public fn legs() -> i32 { this.legs }

        public fn summary() -> String {
            format!("{} ({} legs, age {}): {} meals", this.name, this.legs, this.age, this.meals())
        }
    }
}

let rex = new!(Animal("Rex"));
rex.eat("bone".to_string());
rex.eat("biscuit".to_string());
rex.birthday();
assert_eq!(rex.summary(), "Rex (4 legs, age 1): 2 meals");
assert_eq!(rex.legs(), 4);
```

- 修饰符和 Java 一样：`public`、`protected`、`private`，不写就是包级。顺序随意，比如 `private static`。
- 没有初始值的字段取 `Default::default()`：数字是 0，`bool` 是 `false`，`String` 是 `""`，`Vec` 是空的。
  （Java 的引用字段默认是 `null`；roost 没有 `null`，见第 10 节。）
- **读字段读到的是副本**（`clone`）。对 `i32`、`String`、`Ref<..>` 来说这和 Java 没区别；对 `Vec`、`HashMap`
  这类「值类型」的容器要注意：`this.meals.push(x)` 改的会是副本，所以 roost 让它**编译不通过**，而不是悄悄丢掉
  修改。要改就像上面 `eat` 那样读出来、改、写回。
- 字段只能通过 `this.f` 访问，**不能从类外访问**（即使是 `public` 字段），请写 getter。

## 4. 继承、`super` 和 `@Override`

```rust
use roost::prelude::*;

class! {
    public class Animal {
        protected name: String;

        public Animal(name: String) {
            this.name = name;
            this.init();                  // 虚调用：构造 Dog 时会调到 Dog.init()，和 Java 一样
        }

        public fn speak() -> String { format!("{} makes a sound", this.name) }
        protected fn init() {}
    }

    public class Dog extends Animal {
        private tricks: i32;

        public Dog(name: String) {
            super(name);                  // 必须是构造器的第一条语句
        }

        @Override
        public fn speak() -> String { super.speak() + ", woof" }

        @Override
        protected fn init() { this.tricks = 1; }

        public fn tricks() -> i32 { this.tricks }
    }

    public class Puppy extends Dog {
        public Puppy(name: String) { super(name); }

        @Override
        public fn speak() -> String { format!("{} (tiny)", super.speak()) }
    }
}

let p = new!(Puppy("Bit"));
assert_eq!(p.speak(), "Bit makes a sound, woof (tiny)");
assert_eq!(p.tricks(), 1);
```

- `extends`、`super(..)`、`super.method()` 和 Java 一样，继承层数不限。
- **覆盖父类方法必须写 `@Override`**。Java 里它是可选的，roost 里不写会报错。反过来，写了 `@Override`
  却没有覆盖任何东西（比如方法名拼错）也会报错，并提示你可能想写的名字。
- 父类构造器有参数时，子类构造器必须以 `super(..)` 开头；父类有无参构造器（或没有构造器）时可以省略，
  和 Java 一样会隐式调用 `super()`。
- 每个类**最多一个构造器**（Rust 没有重载）。需要多种构造方式时，加 `public static fn` 工厂方法（第 8 节）。
- 构造顺序和 Java 相同：先分配对象、所有字段取默认值，再从最上层的父类开始依次执行构造器体。所以
  `Animal` 的构造器里调 `this.init()` 会落到子类的覆盖版本。

## 5. 多态：`Ref`、`instanceof`、强制转换

```rust
use roost::prelude::*;

class! {
    public class Animal {
        protected name: String;
        public Animal(name: String) { this.name = name; }
        public fn speak() -> String { format!("{} makes a sound", this.name) }
    }
    public class Dog extends Animal {
        public Dog(name: String) { super(name); }
        @Override public fn speak() -> String { format!("{}: woof", this.name) }
        public fn fetch() -> String { format!("{} fetches the ball", this.name) }
    }
    public class Cat extends Animal {
        public Cat(name: String) { super(name); }
        @Override public fn speak() -> String { format!("{}: meow", this.name) }
    }
}

// Java: List<Animal> zoo = List.of(new Dog("Rex"), new Cat("Tom"));
let zoo: Vec<Ref<Animal>> = vec![new!(Dog("Rex")), new!(Cat("Tom"))];
let sounds: Vec<String> = zoo.iter().map(|a| a.speak()).collect();
assert_eq!(sounds, ["Rex: woof", "Tom: meow"]);

// 向上转型是隐式的，只要目标类型写出来了：
let a: Ref<Animal> = new!(Dog("Ace"));
// 没有目标类型的地方用 `as` 或 `.upcast()`：
let b = new!(Cat("Kit")) as Ref<Animal>;
let c = new!(Cat("Kit")).upcast::<Animal>();

// Java: if (a instanceof Dog) { ((Dog) a).fetch(); }
assert!(a.instanceof::<Dog>());
if let Some(d) = a.cast::<Dog>() {
    assert_eq!(d.fetch(), "Ace fetches the ball");
}
// 转换失败不会抛 ClassCastException，而是得到 None：
assert!(b.cast::<Dog>().is_none());

// Java 的 `==`（是不是同一个对象）：
let a2 = a.clone();
assert!(Ref::ptr_eq(&a, &a2));
assert!(!Ref::ptr_eq(&b, &c));
```

- `Ref<Animal>` 能指向任何 `Animal` 及其子类的对象，方法调用走虚函数表，和 Java 的引用一样。
- `instanceof::<T>()` / `cast::<T>()` 的 `T` 可以是任何类（叶子类、中间类）或接口。
- **`Ref` 不支持 `==`**。比较引用用 `Ref::ptr_eq(&a, &b)`，比较内容用 `a.equals(..)`（第 9 节）。
  这样写的人和读的人都不会把「同一个对象」和「相等」搞混。

## 6. 接口

```rust
use roost::prelude::*;

class! {
    public interface Named {
        fn name() -> String;                       // 抽象方法：没有方法体
    }

    public interface Pet extends Named {           // 接口可以继承多个接口
        fn owner() -> String;

        default fn greet() -> String {             // default 方法
            format!("{} greets {}", this.name(), this.owner())
        }

        static fn kinds() -> i32 { 2 }             // 静态方法：Pet::kinds()
    }

    public class Animal implements Named {
        protected name: String;
        public Animal(name: String) { this.name = name; }
        @Override public fn name() -> String { this.name }   // 实现接口方法也要 @Override
    }

    public class Dog extends Animal implements Pet {
        private owner: String;
        public Dog(name: String, owner: String) { super(name); this.owner = owner; }
        @Override public fn owner() -> String { this.owner }
    }
}

let p: Ref<Pet> = new!(Dog("Rex", "Ann"));
assert_eq!(p.greet(), "Rex greets Ann");
assert_eq!(Pet::kinds(), 2);
assert!(p.instanceof::<Named>());
let a: Ref<Animal> = p.cast::<Animal>().unwrap();     // 接口 → 类
assert_eq!(a.name(), "Rex");
```

- 接口方法都是 `public` 的，可以省略修饰符。
- 一个类可以 `implements` 多个接口：`class Dog extends Animal implements Pet, Comparable`。
- 父类里继承来的方法也能用来实现接口方法（上面的 `name()`），规则和 Java 一样。
- 接口没有字段和构造器。

## 7. 抽象类和 `final`

```rust
use roost::prelude::*;

class! {
    public abstract class Shape {
        public abstract fn area() -> f64;                 // 抽象方法：以 ; 结尾，没有方法体
        public final fn describe() -> String {            // final：子类不能覆盖
            format!("area {:.1}", this.area())
        }
    }

    public final class Square extends Shape {             // final：不能被继承
        private side: f64;
        public Square(side: f64) { this.side = side; }
        @Override public fn area() -> f64 { this.side * this.side }
    }
}

let s: Ref<Shape> = new!(Square(3.0));
assert_eq!(s.describe(), "area 9.0");
```

`new!(Shape())` 是编译错误（`cannot instantiate abstract class`），和 Java 一样。具体类没有实现全部抽象方法、
继承 `final` 类、覆盖 `final` 方法也都是编译错误，报错会指向你写的那一行。

```rust,compile_fail
use roost::prelude::*;

class! {
    public abstract class Shape {
        public abstract fn area() -> f64;
    }
}

let s = new!(Shape()); // error: cannot instantiate abstract class `Shape`
```

## 8. 静态字段和静态方法

```rust
use roost::prelude::*;

class! {
    public class Animal {
        private static count: i32 = 0;    // 静态字段
        protected name: String;

        public Animal(name: String) {
            this.name = name;
            Animal::count += 1;           // Java: Animal.count++（或省略类名）
        }

        public static fn count() -> i32 { Self::count }         // 静态方法：没有 this

        // 静态工厂方法：代替 Java 的构造器重载
        public static fn unnamed() -> Ref<Animal> { Animal::new("(unnamed)") }

        public fn name() -> String { this.name }
    }
}

let a = new!(Animal("Rex"));
let b = Animal::unnamed();
assert_eq!(Animal::count(), 2);
assert_eq!(b.name(), "(unnamed)");
assert_eq!(a.name(), "Rex");
```

- 静态字段在类里写成 `Animal::count` 或 `Self::count`（Java 的 `Animal.count`），读、赋值、`+=` 都可以。
  **只能在类里访问**；类外请写静态 getter，就像上面的 `count()`。
- 静态方法从外面调用写 `Animal::count()`。静态方法里没有 `this`。
- 和 Java 不同：**每个线程有一份静态字段**（垃圾回收器是线程本地的）。单线程程序感觉不到区别。

## 9. `toString`、`equals`、`hashCode`

```rust
use roost::prelude::*;

class! {
    public class Point {
        private x: i32;
        private y: i32;

        public Point(x: i32, y: i32) { this.x = x; this.y = y; }

        public fn x() -> i32 { this.x }
        public fn y() -> i32 { this.y }

        @Override
        public fn to_string() -> String { format!("Point({}, {})", this.x, this.y) }

        @Override
        public fn equals(other: Ref<Object>) -> bool {
            // Java: if (!(other instanceof Point p)) return false;
            match other.cast::<Point>() {
                Some(p) => this.x == p.x() && this.y == p.y(),
                None => false,
            }
        }

        @Override
        public fn hash_code() -> i32 { 31 * this.x + this.y }
    }
}

let a = new!(Point(1, 2));
let b = new!(Point(1, 2));
assert_eq!(format!("{a}"), "Point(1, 2)");     // println!("{a}") 调用 to_string()
assert!(a.equals(b.clone()));                   // 内容相等
assert!(!Ref::ptr_eq(&a, &b));                  // 但不是同一个对象
assert_eq!(a.hash_code(), b.hash_code());
let o: Ref<Object> = a.clone();                 // 任何对象都能当 Object 用
assert_eq!(o.to_string(), "Point(1, 2)");
```

- 方法名用 Rust 的命名：`to_string`、`equals`、`hash_code`。
- `equals` 的参数是 `Ref<Object>`，任何 `Ref` 都能直接传进去；调用时一般写 `b.clone()`（复制的是引用）。
- 没有覆盖时，`to_string()` 是 `类名@十六进制哈希`，`equals` 比较是不是同一个对象，和 Java 的默认实现一样。
- 覆盖里可以用 `super.to_string()` 等调父类（或 `Object`）的版本。

## 10. `null` 去哪了：可空引用和对象环

roost（和 Rust）没有 `null`。一个「可能没有」的引用写成 `Option<Ref<..>>`，值是 `None` 或 `Some(x)`：

```rust
use roost::prelude::*;

class! {
    public class Animal {
        protected name: String;
        private friend: Option<Ref<Animal>>;     // Java: private Animal friend;（默认 null）

        public Animal(name: String) { this.name = name; }

        public fn befriend(other: Ref<Animal>) { this.friend = Some(other); }
        public fn forget() { this.friend = None; }   // Java: this.friend = null;

        public fn friend_name() -> String {
            match this.friend {
                Some(f) => f.name(),
                None => "nobody".to_string(),
            }
        }

        public fn name() -> String { this.name }
    }
}

let rex = new!(Animal("Rex"));
let tom = new!(Animal("Tom"));
assert_eq!(rex.friend_name(), "nobody");

// 互相引用的对象形成一个环：Rust 的引用计数回收不了它，roost 的垃圾回收器可以。
rex.befriend(tom.clone());
tom.befriend(rex.clone());
assert_eq!(rex.friend_name(), "Tom");
drop(rex);
drop(tom);
roost::gc::collect();   // 立即回收（平时不用手动调，堆变大时会自动回收）
```

`this` 本身不能存起来（方法体里它是一个借用，不是 `Ref`），所以环要从外面建，比如上面的
`rex.befriend(tom.clone())`。

## 11. 程序入口 `#[roost::main]`

```rust
use roost::prelude::*;

class! {
    public class Animal {
        protected name: String;
        public Animal(name: String) { this.name = name; }
        public fn name() -> String { this.name }
    }
}

#[roost::main(stack = "256MB", gc_threshold = "1MB", gc_stats)]
fn main() -> Result<(), std::num::ParseIntError> {
    let args: Vec<String> = std::env::args().skip(1).collect();   // Java 的 String[] args
    let count: i32 = args.first().map_or("3", String::as_str).parse()?;
    for i in 0..count {
        println!("{}", new!(Animal(format!("animal #{i}"))).name());
    }
    Ok(())
}
```

属性的参数都是可选的：

| 参数 | 作用 |
| --- | --- |
| `stack = "256MB"` | `main` 所在线程的栈大小，默认 64 MB（深的对象图需要栈来标记）。单位 `B` / `KB` / `MB` / `GB` |
| `gc_threshold = "1MB"` | 堆分配到多少字节时第一次自动回收 |
| `gc_stats` | 退出时在 stderr 打印分配和回收的统计 |

`main` 可以返回 `()` 或 `Result<(), E>`。返回 `Err(e)` 时打印成 Java 未捕获异常的样子，退出码 1：

```text
Exception in thread "main" ParseIntError: invalid digit found in string
```

roost 自己在运行时抛出的异常（目前只有字段访问冲突 `roost.BorrowConflict`，只有在 `Clone` 之类的用户代码
回调同一个对象时才可能发生）也会这样打印，并带上出错的那一行。其他 panic（比如 `unwrap()` 一个 `None`）
和普通 Rust 程序一样，退出码 101。

## 12. 编译器报错时

roost 会尽量用 Java 用户能看懂的话报错，并且指向你写的那一行。例如把父类名拼错：

```rust,compile_fail
use roost::prelude::*;

class! {
    public class Animal {}
    public class Dog extends Animl {}
}
```

```text
error: unknown superclass `Animl`
 --> src/main.rs:5:30
  |
5 |     public class Dog extends Animl {}
  |                              ^^^^^
  |
  = help: did you mean `Animal`?
```

按 Java 习惯写成 `private String name;`：

```rust,compile_fail
use roost::prelude::*;

class! {
    public class Dog {
        private String name;
    }
}
```

```text
error: Java-style field declaration: the type comes after the name
 --> src/main.rs:5:17
  |
5 |         private String name;
  |                 ^^^^^^
  |
  = help: write `name: String;`
```

给 `String` 参数传 `"Rex".into()`：

```rust,compile_fail
use roost::prelude::*;

class! {
    public class Dog {
        private name: String;
        public Dog(name: String) { this.name = name; }
    }
}

let d = Dog::new("Rex".into()); // error[E0283]: type annotations needed
```

去掉 `.into()` 就好：`Dog::new("Rex")`。README 的[常见错误](../README.md#常见编译错误)一节列出了全部
40 多种有专门提示的错误。

## 13. 完整的动物园

把前面的内容放在一起：

```rust
use roost::prelude::*;

class! {
    public interface Named {
        fn name() -> String;
    }

    public interface Pet extends Named {
        fn owner() -> String;
        default fn greet() -> String { format!("{} greets {}", this.name(), this.owner()) }
    }

    public abstract class Animal implements Named {
        private static population: i32 = 0;
        protected name: String;
        private meals: Vec<String>;
        private friend: Option<Ref<Animal>>;

        protected Animal(name: String) {
            this.name = name;
            Animal::population += 1;
        }

        public abstract fn sound() -> String;

        public final fn speak() -> String { format!("{} says {}", this.name, this.sound()) }

        public fn eat(food: String) {
            let mut meals = this.meals;
            meals.push(food);
            this.meals = meals;
        }

        public fn meals() -> usize { this.meals.len() }

        public fn befriend(other: Ref<Animal>) { this.friend = Some(other); }

        public fn friend_name() -> String {
            match this.friend {
                Some(f) => f.name(),
                None => "nobody".to_string(),
            }
        }

        public static fn population() -> i32 { Self::population }

        @Override public fn name() -> String { this.name }

        @Override public fn to_string() -> String { format!("{}({})", this.class_name(), this.name) }
    }

    public class Dog extends Animal implements Pet {
        private owner: String;
        private tricks: i32;

        public Dog(name: String, owner: String) {
            super(name);
            this.owner = owner;
        }

        @Override public fn sound() -> String { "woof".to_string() }
        @Override public fn owner() -> String { this.owner }

        public fn learn() { this.tricks += 1; }
        public fn tricks() -> i32 { this.tricks }
    }

    public final class Puppy extends Dog {
        public Puppy(name: String, owner: String) { super(name, owner); }
        @Override public fn sound() -> String { format!("{} (squeaky)", super.sound()) }
    }

    public class Cat extends Animal {
        public Cat(name: String) { super(name); }
        @Override public fn sound() -> String { "meow".to_string() }
    }
}

#[roost::main(gc_stats)]
fn main() {
    let rex = new!(Dog("Rex", "Ann"));
    let bit = new!(Puppy("Bit", "Bob"));
    let tom = new!(Cat("Tom"));

    let zoo: Vec<Ref<Animal>> = vec![rex.clone(), bit.clone(), tom.clone()];
    for a in &zoo {
        println!("{a}: {}", a.speak());
    }

    rex.learn();
    rex.eat("bone".to_string());
    println!("{} knows {} trick(s) and had {} meal(s)", rex.name(), rex.tricks(), rex.meals());

    let pets: Vec<Ref<Pet>> = zoo.iter().filter_map(|a| a.cast::<Pet>()).collect();
    for p in &pets {
        println!("{}", p.greet());
    }

    for a in &zoo {
        match a.cast::<Dog>() {
            Some(d) => println!("{} is a dog with {} trick(s)", d.name(), d.tricks()),
            None => println!("{} is not a dog", a.name()),
        }
    }

    rex.befriend(tom.clone());
    tom.befriend(rex.clone());
    println!("{}'s friend: {}", rex.name(), rex.friend_name());
    println!("population: {}", Animal::population());
    // rex 和 tom 互相引用，形成一个环；程序退出前 #[roost::main] 会回收它们，gc_stats 打印统计。
}
```

运行它（`cargo run`）会输出：

```text
Dog(Rex): Rex says woof
Puppy(Bit): Bit says woof (squeaky)
Cat(Tom): Tom says meow
Rex knows 1 trick(s) and had 1 meal(s)
Rex greets Ann
Bit greets Bob
Rex is a dog with 1 trick(s)
Bit is a dog with 0 trick(s)
Tom is not a dog
Rex's friend: Tom
population: 3
[roost gc] ...
```

下一步：README 里的 [Java → roost 对照表](../README.md#java--roost-对照表)和[限制与差异](../README.md#限制与差异)。
