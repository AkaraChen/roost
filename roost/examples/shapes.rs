//! `cargo run -p roost --example shapes`: interfaces, abstract classes, static members and
//! the `Object` methods in one screen.

use roost::prelude::*;

class! {
    public interface Named {
        fn name() -> String;
    }

    /// Anything with an area. Extends `Named`, adds a default method.
    public interface Measurable extends Named {
        fn area() -> f64;
        default fn describe() -> String {
            format!("{} with area {:.2}", this.name(), this.area())
        }
    }

    public interface Scalable {
        fn scale(factor: f64);
    }

    public abstract class Shape implements Measurable {
        /// Shapes created so far (on this thread).
        private static count: i32 = 0;
        protected id: i32;

        protected Shape() {
            Shape::count += 1;
            this.id = Shape::count;
        }

        @Override public fn name() -> String { format!("{} #{}", this.kind(), this.id) }

        public abstract fn kind() -> String;

        public static fn count() -> i32 { Self::count }

        @Override
        public fn to_string() -> String { this.describe() }
    }

    public class Circle extends Shape implements Scalable {
        private r: f64;

        public Circle(r: f64) { super(); this.r = r; }

        @Override public fn kind() -> String { "circle".into() }
        @Override public fn area() -> f64 { std::f64::consts::PI * this.r * this.r }
        @Override public fn scale(factor: f64) { this.r *= factor; }
    }

    public final class Rect extends Shape {
        private w: f64;
        private h: f64;

        public Rect(w: f64, h: f64) { super(); this.w = w; this.h = h; }

        @Override public fn kind() -> String { "rect".into() }
        @Override public fn area() -> f64 { this.w * this.h }

        @Override
        public fn equals(other: Ref<Object>) -> bool {
            match other.cast::<Rect>() {
                Some(r) => this.w == r.w() && this.h == r.h(),
                None => false,
            }
        }

        public fn w() -> f64 { this.w }
        public fn h() -> f64 { this.h }
    }
}

fn main() {
    let shapes: Vec<Ref<Shape>> = vec![Circle::new(1.0), Rect::new(2.0, 3.0), Circle::new(0.5)];
    println!("Shape::count()           = {}", Shape::count());

    // Abstract method + interface default method, dispatched through `Ref<Shape>`.
    for s in &shapes {
        println!("{s}"); // Display -> toString() -> describe()
    }

    // Interface references, instanceof and cast.
    let measurable: Vec<Ref<Measurable>> = shapes.iter().map(|s| s.clone().upcast()).collect();
    let total: f64 = measurable.iter().map(|m| m.area()).sum();
    println!("total area               = {total:.2}");
    for s in &shapes {
        if let Some(sc) = s.cast::<Scalable>() {
            sc.scale(2.0);
        }
        println!(
            "{:<10} instanceof Scalable = {:<5} -> area {:.2}",
            s.name(),
            s.instanceof::<Scalable>(),
            s.area()
        );
    }

    // equals vs ptr_eq.
    let a = Rect::new(2.0, 3.0);
    let b = Rect::new(2.0, 3.0);
    println!("a.equals(b)              = {}", a.equals(b.clone()));
    println!("Ref::ptr_eq(&a, &b)      = {}", Ref::ptr_eq(&a, &b));
    println!("Shape::count()           = {}", Shape::count());
}
