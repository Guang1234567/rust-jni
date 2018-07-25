#![allow(dead_code)]
extern crate rust_jni;
extern crate rust_jni_generator;

#[cfg(test)]
mod java {
    pub mod lang {
        pub use rust_jni::java::lang::*;
    }
}

#[cfg(test)]
mod c {
    pub mod d {
        #[allow(unused_imports)]
        use rust_jni_generator::*;

        java_generate! {
            public interface c.d.TestInterface1 {}
            public interface c.d.TestInterface2 extends c.d.TestInterface1 {}

            public class c.d.TestClass1 {}
            public class c.d.TestClass2 extends c.d.TestClass1 implements c.d.TestInterface1 {}
        }
    }
}

#[cfg(test)]
mod a {
    mod b {
        #[allow(unused_imports)]
        use rust_jni_generator::*;

        java_generate! {
            public interface a.b.TestInterface3 {}
            public interface a.b.TestInterface4 extends c.d.TestInterface2, a.b.TestInterface3 {}

            public class a.b.TestClass3 extends c.d.TestClass2 implements c.d.TestInterface1, a.b.TestInterface4 {}

            metadata {
                interface c.d.TestInterface1 {}
                interface c.d.TestInterface2 extends c.d.TestInterface1 {}

                class c.d.TestClass1;
                class c.d.TestClass2 extends c.d.TestClass1 implements c.d.TestInterface1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn test() {}
}
