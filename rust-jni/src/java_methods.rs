use crate::env::JniEnv;
use crate::java_class::find_class;
use crate::java_class::JavaClass;
use crate::java_class::JavaClassExt;
use crate::java_class::JavaClassRef;
use crate::jni_methods;
use crate::jni_types::private::JniArgumentType;
use crate::jni_types::private::JniArgumentTypeTuple;
use crate::object::Object;
use crate::result::JavaResult;
use crate::token::NoException;
use core::ptr::{self, NonNull};

/// A trait to be implemented by all types that can be passed or returned from JNI.
///
/// To pass a type to Java it needs to:
///  1. Be convertible into a type implementing the `JniType` trait: implement `JavaArgumentType` trait
///  2. Provide a JNI signature (see
///     [JNI documentation](https://docs.oracle.com/en/java/javase/11/docs/specs/jni/types.html#type-signatures)
///     for more context).
///
/// To return a type from Java a type also needs to:
///  3. Be convertible from a type implementing the `JniType` trait: implement `JavaMethodResult` trait
///
/// [`rust-jni`](index.html) implements all three conditions for for primitive types that can be passed to JNI.
///
/// Implementing those conditions for Java class wrappers requires cooperation with the wrappers author.
/// [`Object`](java/lang/struct.Object.html) is convertible to and from [`jobject`](../jni_sys/type.jobject.html)
/// which implements the `JniType` trait. So for Java class wrappers the conditions above translate into:
///  1. Be convertible into [`Object`](java/lang/struct.Object.html)
///  2. Provide a JNI signature. For Java classes the signature is `L${CLASS_PATH};`
///  3. Be constructable from [`Object`](java/lang/struct.Object.html)
///
///  - To make a Java class wrapper convertible to [`Object`](java/lang/struct.Object.html) author of the wrapper
///    needs to implement [`AsRef<Object>`](https://doc.rust-lang.org/std/convert/trait.AsRef.html) for it
///  - To make a Java class wrapper constructable from [`Object`](java/lang/struct.Object.html) author of the wrapper
///    needs to implement [`FromObject`](trait.FromObject.html) for it
///  - To provide the JNI signature for a Java class wrapper author needs to implement
///    [`JniSignature`](trait.JniSignature.html)
pub trait JniSignature {
    /// Return the JNI signature for `Self`.
    ///
    /// This method is not unsafe. Returning an incorrect signature will result in a panic, not any unsafe
    /// behaviour.
    fn signature() -> &'static str;
}

impl<T> JniSignature for &'_ T
where
    T: JniSignature,
{
    #[inline(always)]
    fn signature() -> &'static str {
        T::signature()
    }
}

/// A trait for making Java class wrappers constructible from an [`Object`](java/lang/struct.Object.html).
///
/// See more detailed info for passing values betweed Java and rust in
/// [`JniSignature`](trait.JniSignature.html) documentation.
pub trait FromObject<'a> {
    /// Construct `Self` from an [`Object`](java/lang/struct.Object.html).
    ///
    /// Unsafe because it's possible to pass an object of a different type.
    unsafe fn from_object(object: Object<'a>) -> Self;
}

/// A trait that needs to be implemented for a type that needs to be passed to Java.
///
/// See more detailed info for passing values betweed Java and rust in
/// [`JniSignature`](trait.JniSignature.html) documentation.
pub trait JavaArgumentType: JniSignature {
    type JniType: JniArgumentType;

    fn to_jni(&self) -> Self::JniType;
}

impl<'a, T> JavaArgumentType for T
where
    T: JavaClassRef<'a>,
{
    type JniType = jni_sys::jobject;

    #[inline(always)]
    fn to_jni(&self) -> Self::JniType {
        // Safe because we use the pointer only to pass it to JNI.
        unsafe { self.as_ref().raw_object().as_ptr() }
    }
}

impl<'a, T> JniSignature for Option<T>
where
    T: JavaClassRef<'a>,
{
    #[inline(always)]
    fn signature() -> &'static str {
        T::signature()
    }
}

impl<'a, T> JavaArgumentType for Option<T>
where
    T: JavaClassRef<'a>,
{
    type JniType = jni_sys::jobject;

    #[inline(always)]
    fn to_jni(&self) -> Self::JniType {
        // Safe because we use the pointer only to pass it to JNI.
        unsafe {
            self.as_ref().map_or(ptr::null_mut(), |value| {
                value.as_ref().raw_object().as_ptr()
            })
        }
    }
}

pub trait JavaArgumentTuple {
    type JniType: JniArgumentTypeTuple;

    fn to_jni(&self) -> Self::JniType;
}

pub trait JavaMethodSignature<In, Out>
where
    In: JavaArgumentTuple,
{
    fn method_signature() -> std::string::String;
}

macro_rules! braces {
    ($name:ident) => {
        "{}"
    };
}

macro_rules! peel_java_argument_type_impls {
    () => ();
    ($type:ident, $($other:ident,)*) => (java_argument_type_impls! { $($other,)* });
}

macro_rules! java_argument_type_impls {
    ( $($type:ident,)*) => (
        impl<'a, $($type),*> JavaArgumentTuple for ($($type,)*)
        where
            $($type: JavaArgumentType,)*
        {
            type JniType = ($($type::JniType,)*);

            #[inline(always)]
            fn to_jni(&self) -> Self::JniType {
                #[allow(non_snake_case)]
                let ($($type,)*) = self;
                ($($type.to_jni(),)*)
            }
        }

        impl<'a, $($type,)* Out, F> JavaMethodSignature<($($type,)*), Out> for F
            where
                $($type: JavaArgumentType,)*
                Out: JniSignature,
                F: FnOnce($($type,)*) -> Out + ?Sized,
        {
            #[inline(always)]
            fn method_signature() -> std::string::String {
                format!(
                    concat!("(", $(braces!($type), )* "){}\0"),
                    $(<$type as JniSignature>::signature(),)*
                    <Out as JniSignature>::signature(),
                )
            }
        }

        peel_java_argument_type_impls! { $($type,)* }
    );
}

java_argument_type_impls! {
    T0,
    T1,
    T2,
    T3,
    T4,
    T5,
    T6,
    T7,
    T8,
    T9,
    T10,
    T11,
}

/// Call a Java method.
///
/// The method has four generic parameters:
///  - The first one is the class of the object. It doesn't have to be the exact class,
///    a subclass can be passed as well. Can be inferred
///  - The second one is the type of the arguments tuple. Can be inferred
///  - The third one is the Java result type. Can be inferred
///  - The fourth one is the signature of the Java method. Must be specified
///
/// As a result, only one generic parameter needs to be specified -- the last one.
///
/// Example:
/// ```
/// # use rust_jni::*;
/// # use rust_jni::java::lang::String;
/// # use std::ptr;
/// #
/// # fn jni_main<'a>(env: &'a JniEnv<'a>, token: NoException<'a>) -> JavaResult<'a, NoException<'a>> {
/// let object = String::empty(env, &token)?;
/// // Safe because correct arguments are passed and correct return type specified.
/// // See `Object::hashCode` javadoc:
/// // https://docs.oracle.com/javase/10/docs/api/java/lang/Object.html#hashCode()
/// let hash_code = unsafe {
///     call_method::<_, _, _, fn() -> i32>(&object, &token, "hashCode\0", ())
/// }?;
/// assert_eq!(hash_code, 0);
/// # Ok(token)
/// # }
/// #
/// # fn main() {
/// #     let init_arguments = InitArguments::default();
/// #     let vm = JavaVM::create(&init_arguments).unwrap();
/// #     let _ = vm.with_attached(
/// #        &AttachArguments::new(init_arguments.version()),
/// #        |env: &JniEnv, token: NoException| {
/// #            ((), jni_main(env, token).unwrap())
/// #        },
/// #     );
/// # }
/// ```
///
/// Note that method name string *must* be null-terminating.
///
/// See more info about how to pass or return types from Java calls in [`JniSignature`](trait.JniSignature.html)
/// documentation
///
/// This method is unsafe because incorrect parameters can be passed to a method or incorrect return type specified.
pub unsafe fn call_method<'a, T, A, R, F>(
    object: &T,
    token: &NoException<'a>,
    name: &str,
    arguments: A,
) -> JavaResult<'a, R::ResultType>
where
    T: JavaClassRef<'a>,
    A: JavaArgumentTuple,
    R: JavaMethodResult<'a>,
    F: JavaMethodSignature<A, R>,
{
    R::call_method::<T, A>(object, token, name, &F::method_signature(), arguments)
}

/// Call a static Java method.
///
/// The method has four generic parameters:
///  - The first one is the class of the object. Can be inferred
///  - The second one is the type of the arguments tuple. Can be inferred
///  - The third one is the Java result type. Can be inferred
///  - The fourth one is the signature of the Java method. Must be specified
///
/// As a result, only one generic parameter needs to be specified -- the last one.
///
/// Example:
/// ```
/// # use rust_jni::*;
/// # use rust_jni::java::lang::String;
/// # use std::ptr;
/// #
/// # fn jni_main<'a>(env: &'a JniEnv<'a>, token: NoException<'a>) -> JavaResult<'a, NoException<'a>> {
/// // Safe because correct arguments are passed and correct return type specified.
/// // See `String::valueOf(int)` javadoc:
/// // https://docs.oracle.com/javase/10/docs/api/java/lang/String.html#valueOf(int)
/// let string_value = unsafe {
///     call_static_method::<String<'a>, _, _, fn(i32) -> String<'a>>(
///         env,
///         &token,
///         "valueOf\0",
///         (17,),
///     )
/// }
/// .or_npe(env, &token)?
/// .as_string(&token);
/// assert_eq!(string_value, "17");
/// # Ok(token)
/// # }
/// #
/// # fn main() {
/// #     let init_arguments = InitArguments::default();
/// #     let vm = JavaVM::create(&init_arguments).unwrap();
/// #     let _ = vm.with_attached(
/// #        &AttachArguments::new(init_arguments.version()),
/// #        |env: &JniEnv, token: NoException| {
/// #            ((), jni_main(env, token).unwrap())
/// #        },
/// #     );
/// # }
/// ```
///
/// Note that method name string must be null-terminating.
///
/// See more info about how to pass or return types from Java calls in [`JniSignature`](trait.JniSignature.html)
/// documentation
///
/// This method is unsafe because incorrect parameters can be passed to a method or incorrect return type specified.
pub unsafe fn call_static_method<'a, T, A, R, F>(
    env: &'a JniEnv<'a>,
    token: &NoException<'a>,
    name: &str,
    arguments: A,
) -> JavaResult<'a, R::ResultType>
where
    T: JavaClassRef<'a>,
    A: JavaArgumentTuple,
    R: JavaMethodResult<'a>,
    F: JavaMethodSignature<A, R>,
{
    R::call_static_method::<T, A>(env, token, name, &F::method_signature(), arguments)
}

/// Call a Java constructor
///
/// The method has three generic parameters:
///  - The first one is the class of the object
///  - The second one is the type of the arguments tuple. Can be inferred
///  - The third one is the signature of the Java method. Can be inferred
///
/// As a result, only one generic parameter needs to be specified -- the class type.
///
/// Example:
/// ```
/// # use rust_jni::*;
/// # use rust_jni::java::lang::String;
/// # use std::ptr;
/// #
/// # fn jni_main<'a>(env: &'a JniEnv<'a>, token: NoException<'a>) -> JavaResult<'a, NoException<'a>> {
/// // Safe because correct arguments are passed.
/// // See `String()` javadoc:
/// // https://docs.oracle.com/javase/10/docs/api/java/lang/String.html#<init>()
/// let empty_string = unsafe {
///     call_constructor::<String<'a>, _, fn()>(env, &token, ())
/// }?
/// .as_string(&token);
/// assert_eq!(empty_string, "");
/// # Ok(token)
/// # }
/// #
/// # fn main() {
/// #     let init_arguments = InitArguments::default();
/// #     let vm = JavaVM::create(&init_arguments).unwrap();
/// #     let _ = vm.with_attached(
/// #        &AttachArguments::new(init_arguments.version()),
/// #        |env: &JniEnv, token: NoException| {
/// #            ((), jni_main(env, token).unwrap())
/// #        },
/// #     );
/// # }
/// ```
///
/// See more info about how to pass or return types from Java calls in [`JniSignature`](trait.JniSignature.html)
/// documentation
///
/// This method is unsafe because incorrect parameters can be passed to a method.
pub unsafe fn call_constructor<'a, R, A, F>(
    env: &'a JniEnv<'a>,
    token: &NoException<'a>,
    arguments: A,
) -> JavaResult<'a, R>
where
    A: JavaArgumentTuple,
    R: JavaClass<'a>,
    F: JavaMethodSignature<A, ()>,
{
    let class = R::class(env, token)?;
    let result = jni_methods::call_constructor(
        &class,
        token,
        &F::method_signature(),
        JavaArgumentTuple::to_jni(&arguments),
    )?;
    Ok(R::from_object(Object::from_raw(env, result)))
}

pub trait JavaMethodResult<'a> {
    type JniType;
    type ResultType: 'a;

    unsafe fn call_method<T, A>(
        object: &T,
        token: &NoException<'a>,
        name: &str,
        signature: &str,
        arguments: A,
    ) -> JavaResult<'a, Self::ResultType>
    where
        T: JavaClassRef<'a>,
        A: JavaArgumentTuple;

    unsafe fn call_static_method<T, A>(
        env: &'a JniEnv<'a>,
        token: &NoException<'a>,
        name: &str,
        signature: &str,
        arguments: A,
    ) -> JavaResult<'a, Self::ResultType>
    where
        T: JavaClassRef<'a>,
        A: JavaArgumentTuple;
}

impl<'a, S> JavaMethodResult<'a> for S
where
    S: JavaClass<'a> + 'a,
{
    type JniType = Option<NonNull<jni_sys::_jobject>>;
    type ResultType = Option<Self>;

    #[inline(always)]
    unsafe fn call_method<T, A>(
        object: &T,
        token: &NoException<'a>,
        name: &str,
        signature: &str,
        arguments: A,
    ) -> JavaResult<'a, Self::ResultType>
    where
        T: JavaClassRef<'a>,
        A: JavaArgumentTuple,
    {
        let result = jni_methods::call_object_method(
            object.as_ref(),
            token,
            name,
            signature,
            JavaArgumentTuple::to_jni(&arguments),
        )?;
        Ok(result.map(
            #[inline(always)]
            |result| Self::from_object(Object::from_raw(object.as_ref().env(), result)),
        ))
    }

    #[inline(always)]
    unsafe fn call_static_method<T, A>(
        env: &'a JniEnv<'a>,
        token: &NoException<'a>,
        name: &str,
        signature: &str,
        arguments: A,
    ) -> JavaResult<'a, Self::ResultType>
    where
        T: JavaClassRef<'a>,
        A: JavaArgumentTuple,
    {
        let class = find_class::<T>(env, token)?;
        let result = jni_methods::call_static_object_method(
            &class,
            token,
            name,
            signature,
            JavaArgumentTuple::to_jni(&arguments),
        )?;
        Ok(result.map(
            #[inline(always)]
            |result| Self::from_object(Object::from_raw(env, result)),
        ))
    }
}
