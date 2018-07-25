#![recursion_limit = "1024"]

extern crate proc_macro;
#[macro_use]
extern crate quote;
extern crate proc_macro2;

use proc_macro2::*;
use quote::ToTokens;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::iter::{self, FromIterator};
use std::ops::Deref;

/// Generate `rust-jni` wrappers for Java classes and interfaces.
///
/// TODO(#76): examples.
#[proc_macro]
pub fn java_generate(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let input: TokenStream = input.into();
    java_generate_impl(input).into()
}

fn java_generate_impl(input: TokenStream) -> TokenStream {
    generate(to_generator_data(parse_java_definition(input)))
}

#[derive(Debug, Clone)]
struct JavaName(TokenStream);

impl Deref for JavaName {
    type Target = TokenStream;

    fn deref(&self) -> &TokenStream {
        &self.0
    }
}

impl ToTokens for JavaName {
    fn to_tokens(&self, stream: &mut TokenStream) {
        self.0.to_tokens(stream)
    }
}

impl Hash for JavaName {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.to_string().hash(state);
    }
}

impl PartialEq for JavaName {
    fn eq(&self, other: &Self) -> bool {
        format!("{:?}", self) == format!("{:?}", other)
    }
}

impl Eq for JavaName {}

#[must_use = "iterator adaptors are lazy and do nothing unless consumed"]
struct FlatMapThreaded<I, F, S> {
    iterator: I,
    function: F,
    state: S,
}

impl<I, F, S, T> Iterator for FlatMapThreaded<I, F, S>
where
    I: Iterator<Item = T>,
    F: FnMut(&T, &S) -> S,
{
    type Item = T;

    fn next(&mut self) -> Option<T> {
        match self.iterator.next() {
            None => None,
            Some(value) => {
                self.state = (self.function)(&value, &self.state);
                Some(value)
            }
        }
    }
}

fn flat_map_threaded<I, T, F, S>(iterator: I, initial: S, function: F) -> FlatMapThreaded<I, F, S>
where
    I: Iterator<Item = T>,
    F: FnMut(&T, &S) -> S,
{
    FlatMapThreaded {
        iterator,
        function,
        state: initial,
    }
}

impl JavaName {
    fn from_tokens<'a>(tokens: impl Iterator<Item = &'a TokenTree>) -> JavaName {
        let tokens = flat_map_threaded(tokens, false, |token, was_identifier| {
            match (token, was_identifier) {
                (TokenTree::Ident(_), false) => true,
                (TokenTree::Punct(punct), true) => {
                    if punct.as_char() != '.' {
                        panic!("Expected a dot, got {:?}.", punct);
                    }
                    false
                }
                (token, true) => {
                    panic!("Expected a dot, got {:?}.", token);
                }
                (token, false) => {
                    panic!("Expected an identifier, got {:?}.", token);
                }
            }
        }).filter(|token| match token {
            TokenTree::Ident(_) => true,
            _ => false,
        });
        let tokens = TokenStream::from_iter(tokens.cloned());
        if tokens.is_empty() {
            panic!("Expected a Java name, got no tokens.");
        }
        JavaName(tokens)
    }

    fn name(self) -> Ident {
        match self.0.into_iter().last().unwrap() {
            TokenTree::Ident(identifier) => identifier,
            token => panic!("Expected an identifier, got {:?}", token),
        }
    }

    fn with_slashes(self) -> String {
        self.0
            .into_iter()
            .map(|token| token.to_string())
            .collect::<Vec<_>>()
            .join("/")
    }

    fn with_double_colons(self) -> TokenStream {
        let mut tokens = vec![];
        for token in self.0.into_iter() {
            tokens.extend(quote!{::});
            tokens.push(token);
        }
        TokenStream::from_iter(tokens.iter().cloned())
    }

    fn with_dots(self) -> TokenStream {
        let mut tokens = vec![];
        let mut first = true;
        for token in self.0.into_iter() {
            if first {
                first = false;
            } else {
                tokens.extend(quote!{.});
            }
            tokens.push(token);
        }
        TokenStream::from_iter(tokens.iter().cloned())
    }

    fn as_primitive_type(&self) -> Option<TokenStream> {
        let tokens = self.clone().0.into_iter().collect::<Vec<_>>();
        if tokens.len() == 1 {
            let token = &tokens[0];
            if is_identifier(&token, "int") {
                Some(quote!{i32})
            } else if is_identifier(&token, "long") {
                Some(quote!{i64})
            } else if is_identifier(&token, "char") {
                Some(quote!{char})
            } else if is_identifier(&token, "byte") {
                Some(quote!{u8})
            } else if is_identifier(&token, "boolean") {
                Some(quote!{bool})
            } else if is_identifier(&token, "float") {
                Some(quote!{f32})
            } else if is_identifier(&token, "double") {
                Some(quote!{f64})
            } else {
                None
            }
        } else {
            None
        }
    }

    fn as_rust_type(self) -> TokenStream {
        let primitive = self.as_primitive_type();
        let with_double_colons = self.with_double_colons();
        primitive.unwrap_or(quote!{#with_double_colons <'a>})
    }

    fn as_rust_type_reference(self) -> TokenStream {
        let primitive = self.as_primitive_type();
        let with_double_colons = self.with_double_colons();
        primitive.unwrap_or(quote!{& #with_double_colons})
    }
}

#[derive(Debug, PartialEq, Eq, Clone)]
struct MethodArgument {
    name: Ident,
    data_type: JavaName,
}

#[derive(Debug, PartialEq, Eq, Clone)]
struct JavaClassMethod {
    name: Ident,
    return_type: JavaName,
    arguments: Vec<MethodArgument>,
    public: bool,
    is_static: bool,
}

#[derive(Debug, PartialEq, Eq, Clone)]
struct JavaConstructor {
    arguments: Vec<MethodArgument>,
    public: bool,
}

#[derive(Debug, PartialEq, Eq, Clone)]
struct JavaClass {
    extends: Option<JavaName>,
    implements: Vec<JavaName>,
    methods: Vec<JavaClassMethod>,
    constructors: Vec<JavaConstructor>,
}

#[derive(Debug, PartialEq, Eq, Clone)]
struct JavaInterface {
    extends: Vec<JavaName>,
}

#[derive(Debug, PartialEq, Eq, Clone)]
enum JavaDefinitionKind {
    Class(JavaClass),
    Interface(JavaInterface),
}

#[derive(Debug, PartialEq, Eq, Clone)]
struct JavaDefinition {
    name: JavaName,
    public: bool,
    definition: JavaDefinitionKind,
}

#[derive(Debug, PartialEq, Eq, Clone)]
struct JavaClassMetadata {
    extends: Option<JavaName>,
    implements: Vec<JavaName>,
}

#[derive(Debug, PartialEq, Eq, Clone)]
struct JavaInterfaceMetadata {
    extends: Vec<JavaName>,
}

#[derive(Debug, PartialEq, Eq, Clone)]
enum JavaDefinitionMetadataKind {
    Class(JavaClassMetadata),
    Interface(JavaInterfaceMetadata),
}

#[derive(Debug, PartialEq, Eq, Clone)]
struct JavaDefinitionMetadata {
    name: JavaName,
    definition: JavaDefinitionMetadataKind,
}

#[derive(Debug, PartialEq, Eq, Clone)]
struct Metadata {
    definitions: Vec<JavaDefinitionMetadata>,
}

#[derive(Debug, PartialEq, Eq, Clone)]
struct JavaDefinitions {
    definitions: Vec<JavaDefinition>,
    metadata: Metadata,
}

fn comma_separated_names(tokens: impl Iterator<Item = TokenTree>) -> Vec<JavaName> {
    let tokens = tokens.collect::<Vec<_>>();
    tokens
        .split(|token| match token {
            TokenTree::Punct(punct) => punct.spacing() == Spacing::Alone && punct.as_char() == ',',
            _ => false,
        })
        .filter(|slice| slice.len() > 0)
        .map(|slice| JavaName::from_tokens(slice.iter()))
        .collect()
}

fn parse_interface_header(header: &[TokenTree]) -> (JavaName, Vec<JavaName>) {
    let name = JavaName::from_tokens(
        header
            .iter()
            .take_while(|token| !is_identifier(&token, "extends")),
    );
    let extends = comma_separated_names(
        header
            .iter()
            .skip_while(|token| !is_identifier(&token, "extends"))
            .skip(1)
            .cloned(),
    );
    (name, extends)
}

fn parse_class_header(header: &[TokenTree]) -> (JavaName, Option<JavaName>, Vec<JavaName>) {
    let name = JavaName::from_tokens(header.iter().take_while(|token| {
        !is_identifier(&token, "extends") && !is_identifier(&token, "implements")
    }));
    let implements = comma_separated_names(
        header
            .iter()
            .skip_while(|token| !is_identifier(&token, "implements"))
            .skip(1)
            .cloned(),
    );
    let has_extends = header
        .iter()
        .filter(|token| is_identifier(&token, "extends"))
        .next()
        .is_some();
    let extends = if has_extends {
        Some(JavaName::from_tokens(
            header
                .iter()
                .skip_while(|token| !is_identifier(&token, "extends"))
                .skip(1)
                .take_while(|token| !is_identifier(&token, "implements")),
        ))
    } else {
        None
    };
    (name, extends, implements)
}

fn parse_metadata(tokens: TokenStream) -> Metadata {
    let definitions = tokens.clone().into_iter().collect::<Vec<_>>();
    let definitions = definitions
        .split(is_metadata_definition)
        .filter(|tokens| !tokens.is_empty())
        .map(|header| {
            let (token, header) = header.split_first().unwrap();
            let is_class = is_identifier(&token, "class");
            let is_interface = is_identifier(&token, "interface");
            if !is_class && !is_interface {
                panic!("Expected \"class\" or \"interface\", got {:?}.", token);
            }

            if is_interface {
                let (name, extends) = parse_interface_header(header);
                JavaDefinitionMetadata {
                    name,
                    definition: JavaDefinitionMetadataKind::Interface(JavaInterfaceMetadata {
                        extends,
                    }),
                }
            } else {
                let (name, extends, implements) = parse_class_header(header);
                JavaDefinitionMetadata {
                    name,
                    definition: JavaDefinitionMetadataKind::Class(JavaClassMetadata {
                        extends,
                        implements,
                    }),
                }
            }
        })
        .collect::<Vec<_>>();
    Metadata { definitions }
}

fn is_constructor(tokens: &[TokenTree], class_name: &JavaName) -> bool {
    let public = tokens.iter().any(|token| is_identifier(token, "public"));
    let tokens = if public {
        &tokens[1..tokens.len() - 1]
    } else {
        &tokens[0..tokens.len() - 1]
    };
    TokenStream::from_iter(tokens.iter().cloned()).to_string()
        == class_name.clone().with_dots().to_string()
}

fn parse_method_arguments(token: TokenTree) -> Vec<MethodArgument> {
    match token {
        TokenTree::Group(group) => {
            if group.delimiter() != Delimiter::Parenthesis {
                panic!("Expected method arguments in parenthesis, got {:?}.", group);
            }
            let arguments = group.stream().into_iter().collect::<Vec<_>>();
            arguments
                .split(|token| is_punctuation(token, ','))
                .filter(|tokens| !tokens.is_empty())
                .map(|tokens| tokens.split_last().unwrap())
                .map(|(last, others)| {
                    let name = match last {
                        TokenTree::Ident(ident) => ident.clone(),
                        token => panic!("Expected argument name, got {:?}.", token),
                    };
                    MethodArgument {
                        name,
                        data_type: JavaName::from_tokens(others.iter()),
                    }
                })
                .collect::<Vec<_>>()
        }
        token => panic!("Expected method arguments, got {:?}.", token),
    }
}

fn parse_method(tokens: &[TokenTree]) -> JavaClassMethod {
    let public = tokens.iter().any(|token| is_identifier(token, "public"));
    let is_static = tokens.iter().any(|token| is_identifier(token, "static"));
    let tokens = tokens
        .iter()
        .filter(|token| !is_identifier(token, "public") && !is_identifier(token, "static"))
        .cloned()
        .collect::<Vec<_>>();
    let name = match tokens[tokens.len() - 2].clone() {
        TokenTree::Ident(ident) => ident,
        token => panic!("Expected method name, got {:?}.", token),
    };
    let return_type = JavaName::from_tokens(tokens[0..tokens.len() - 2].iter());
    let arguments = parse_method_arguments(tokens[tokens.len() - 1].clone());
    JavaClassMethod {
        public,
        name,
        return_type,
        arguments,
        is_static,
    }
}

fn parse_constructor(tokens: &[TokenTree]) -> JavaConstructor {
    let public = tokens.iter().any(|token| is_identifier(token, "public"));
    let tokens = tokens
        .iter()
        .filter(|token| !is_identifier(token, "public"))
        .cloned()
        .collect::<Vec<_>>();
    let arguments = parse_method_arguments(tokens[tokens.len() - 1].clone());
    JavaConstructor { public, arguments }
}

fn parse_java_definition(input: TokenStream) -> JavaDefinitions {
    let mut definitions = input.clone().into_iter().collect::<Vec<_>>();
    let metadata = if definitions.len() > 1
        && is_identifier(&definitions[definitions.len() - 2], "metadata")
    {
        match definitions.pop().unwrap() {
            TokenTree::Group(group) => {
                if group.delimiter() == Delimiter::Brace {
                    let metadata = parse_metadata(group.stream());
                    definitions.pop().unwrap();
                    metadata
                } else {
                    panic!("Expected braces, got {:?}.", group)
                }
            }
            token => panic!("Expected braces, got {:?}.", token),
        }
    } else {
        Metadata {
            definitions: vec![],
        }
    };
    let definitions = definitions
        .split(is_definition)
        .filter(|tokens| !tokens.is_empty())
        .map(|header| {
            let (token, header) = header.split_first().unwrap();
            let public = is_identifier(&token, "public");
            let (token, header) = if public {
                header.split_first().unwrap()
            } else {
                (token, header)
            };
            let is_class = is_identifier(&token, "class");
            let is_interface = is_identifier(&token, "interface");
            if !is_class && !is_interface {
                panic!("Expected \"class\" or \"interface\", got {:?}.", token);
            }

            if is_interface {
                let (name, extends) = parse_interface_header(header);
                JavaDefinition {
                    name,
                    public,
                    definition: JavaDefinitionKind::Interface(JavaInterface { extends }),
                }
            } else {
                let (name, extends, implements) = parse_class_header(header);
                JavaDefinition {
                    name,
                    public,
                    definition: JavaDefinitionKind::Class(JavaClass {
                        extends,
                        implements,
                        methods: vec![],
                        constructors: vec![],
                    }),
                }
            }
        })
        .zip(definitions.iter().cloned().filter(is_definition))
        .map(|(definition, token)| match token {
            TokenTree::Group(group) => (definition, group.stream()),
            _ => unreachable!(),
        })
        .map(|(definition, tokens)| {
            let methods = tokens.into_iter().collect::<Vec<_>>();
            let java_definition = match definition.definition.clone() {
                JavaDefinitionKind::Class(class) => {
                    let constructors = methods
                        .split(|token| is_punctuation(token, ';'))
                        .filter(|tokens| !tokens.is_empty())
                        .filter(|tokens| is_constructor(tokens, &definition.name))
                        .map(parse_constructor)
                        .collect::<Vec<_>>();
                    let methods = methods
                        .split(|token| is_punctuation(token, ';'))
                        .filter(|tokens| !tokens.is_empty())
                        .filter(|tokens| !is_constructor(tokens, &definition.name))
                        .map(parse_method)
                        .collect::<Vec<_>>();
                    JavaDefinitionKind::Class(JavaClass {
                        methods,
                        constructors,
                        ..class
                    })
                }
                JavaDefinitionKind::Interface(interface) => {
                    JavaDefinitionKind::Interface(JavaInterface { ..interface })
                }
            };
            JavaDefinition {
                definition: java_definition,
                ..definition
            }
        })
        .collect();
    JavaDefinitions {
        definitions,
        metadata,
    }
}

fn is_punctuation(token: &TokenTree, value: char) -> bool {
    match token {
        TokenTree::Punct(punct) => punct.spacing() == Spacing::Alone && punct.as_char() == value,
        _ => false,
    }
}

fn is_identifier(token: &TokenTree, name: &str) -> bool {
    match token {
        TokenTree::Ident(identifier) => identifier == name,
        _ => false,
    }
}

fn is_definition(token: &TokenTree) -> bool {
    match token {
        TokenTree::Group(group) => group.delimiter() == Delimiter::Brace,
        _ => false,
    }
}

fn is_metadata_definition(token: &TokenTree) -> bool {
    match token {
        TokenTree::Group(group) => group.delimiter() == Delimiter::Brace,
        TokenTree::Punct(puntuation) => puntuation.as_char() == ';',
        _ => false,
    }
}

#[cfg(test)]
mod parse_tests {
    use super::*;

    #[test]
    fn empty() {
        let input = quote!{};
        assert_eq!(
            parse_java_definition(input),
            JavaDefinitions {
                definitions: vec![],
                metadata: Metadata {
                    definitions: vec![],
                },
            }
        );
    }

    #[test]
    fn one_class() {
        let input = quote!{
            class TestClass1 {}
        };
        assert_eq!(
            parse_java_definition(input),
            JavaDefinitions {
                definitions: vec![JavaDefinition {
                    name: JavaName(quote!{TestClass1}),
                    public: false,
                    definition: JavaDefinitionKind::Class(JavaClass {
                        extends: None,
                        implements: vec![],
                        methods: vec![],
                        constructors: vec![],
                    }),
                }],
                metadata: Metadata {
                    definitions: vec![],
                },
            }
        );
    }

    #[test]
    fn one_class_extends() {
        let input = quote!{
            class TestClass1 extends test1 {}
        };
        assert_eq!(
            parse_java_definition(input),
            JavaDefinitions {
                definitions: vec![JavaDefinition {
                    name: JavaName(quote!{TestClass1}),
                    public: false,
                    definition: JavaDefinitionKind::Class(JavaClass {
                        extends: Some(JavaName(quote!{test1})),
                        implements: vec![],
                        methods: vec![],
                        constructors: vec![],
                    }),
                }],
                metadata: Metadata {
                    definitions: vec![],
                },
            }
        );
    }

    #[test]
    fn one_class_public() {
        let input = quote!{
            public class TestClass1 {}
        };
        assert_eq!(
            parse_java_definition(input),
            JavaDefinitions {
                definitions: vec![JavaDefinition {
                    name: JavaName(quote!{TestClass1}),
                    public: true,
                    definition: JavaDefinitionKind::Class(JavaClass {
                        extends: None,
                        implements: vec![],
                        methods: vec![],
                        constructors: vec![],
                    }),
                }],
                metadata: Metadata {
                    definitions: vec![],
                },
            }
        );
    }

    #[test]
    fn one_class_packaged() {
        let input = quote!{
            class a.b.TestClass1 {}
        };
        assert_eq!(
            parse_java_definition(input),
            JavaDefinitions {
                definitions: vec![JavaDefinition {
                    name: JavaName(quote!{a b TestClass1}),
                    public: false,
                    definition: JavaDefinitionKind::Class(JavaClass {
                        extends: None,
                        implements: vec![],
                        methods: vec![],
                        constructors: vec![],
                    }),
                }],
                metadata: Metadata {
                    definitions: vec![],
                },
            }
        );
    }

    #[test]
    fn one_class_implements() {
        let input = quote!{
            class TestClass1 implements test2, a.b.test3 {}
        };
        assert_eq!(
            parse_java_definition(input),
            JavaDefinitions {
                definitions: vec![JavaDefinition {
                    name: JavaName(quote!{TestClass1}),
                    public: false,
                    definition: JavaDefinitionKind::Class(JavaClass {
                        extends: None,
                        implements: vec![JavaName(quote!{test2}), JavaName(quote!{a b test3})],
                        methods: vec![],
                        constructors: vec![],
                    }),
                }],
                metadata: Metadata {
                    definitions: vec![],
                },
            }
        );
    }

    #[test]
    fn one_interface() {
        let input = quote!{
            interface TestInterface1 {}
        };
        assert_eq!(
            parse_java_definition(input),
            JavaDefinitions {
                definitions: vec![JavaDefinition {
                    name: JavaName(quote!{TestInterface1}),
                    public: false,
                    definition: JavaDefinitionKind::Interface(JavaInterface { extends: vec![] }),
                }],
                metadata: Metadata {
                    definitions: vec![],
                },
            }
        );
    }

    #[test]
    fn one_interface_public() {
        let input = quote!{
            public interface TestInterface1 {}
        };
        assert_eq!(
            parse_java_definition(input),
            JavaDefinitions {
                definitions: vec![JavaDefinition {
                    name: JavaName(quote!{TestInterface1}),
                    public: true,
                    definition: JavaDefinitionKind::Interface(JavaInterface { extends: vec![] }),
                }],
                metadata: Metadata {
                    definitions: vec![],
                },
            }
        );
    }

    #[test]
    fn one_interface_packaged() {
        let input = quote!{
            interface a.b.TestInterface1 {}
        };
        assert_eq!(
            parse_java_definition(input),
            JavaDefinitions {
                definitions: vec![JavaDefinition {
                    name: JavaName(quote!{a b TestInterface1}),
                    public: false,
                    definition: JavaDefinitionKind::Interface(JavaInterface { extends: vec![] }),
                }],
                metadata: Metadata {
                    definitions: vec![],
                },
            }
        );
    }

    #[test]
    fn one_interface_extends() {
        let input = quote!{
            interface TestInterface1 extends TestInterface2, a.b.TestInterface3 {}
        };
        assert_eq!(
            parse_java_definition(input),
            JavaDefinitions {
                definitions: vec![JavaDefinition {
                    name: JavaName(quote!{TestInterface1}),
                    public: false,
                    definition: JavaDefinitionKind::Interface(JavaInterface {
                        extends: vec![
                            JavaName(quote!{TestInterface2}),
                            JavaName(quote!{a b TestInterface3}),
                        ],
                    }),
                }],
                metadata: Metadata {
                    definitions: vec![],
                },
            }
        );
    }

    #[test]
    fn multiple() {
        let input = quote!{
            interface TestInterface1 {}
            interface TestInterface2 {}
            class TestClass1 {}
            class TestClass2 {}
        };
        assert_eq!(
            parse_java_definition(input),
            JavaDefinitions {
                definitions: vec![
                    JavaDefinition {
                        name: JavaName(quote!{TestInterface1}),
                        public: false,
                        definition: JavaDefinitionKind::Interface(JavaInterface {
                            extends: vec![],
                        }),
                    },
                    JavaDefinition {
                        name: JavaName(quote!{TestInterface2}),
                        public: false,
                        definition: JavaDefinitionKind::Interface(JavaInterface {
                            extends: vec![],
                        }),
                    },
                    JavaDefinition {
                        name: JavaName(quote!{TestClass1}),
                        public: false,
                        definition: JavaDefinitionKind::Class(JavaClass {
                            extends: None,
                            implements: vec![],
                            methods: vec![],
                            constructors: vec![],
                        }),
                    },
                    JavaDefinition {
                        name: JavaName(quote!{TestClass2}),
                        public: false,
                        definition: JavaDefinitionKind::Class(JavaClass {
                            extends: None,
                            implements: vec![],
                            methods: vec![],
                            constructors: vec![],
                        }),
                    },
                ],
                metadata: Metadata {
                    definitions: vec![],
                },
            }
        );
    }

    #[test]
    fn metadata_empty() {
        let input = quote!{
            metadata {}
        };
        assert_eq!(
            parse_java_definition(input),
            JavaDefinitions {
                definitions: vec![],
                metadata: Metadata {
                    definitions: vec![],
                },
            }
        );
    }

    #[test]
    fn metadata() {
        let input = quote!{
            metadata {
                interface TestInterface1 {}
                interface TestInterface2 extends TestInterface1 {}
                class TestClass2;
                class TestClass1 extends TestClass2 implements TestInterface1, TestInterface2;
            }
        };
        assert_eq!(
            parse_java_definition(input),
            JavaDefinitions {
                definitions: vec![],
                metadata: Metadata {
                    definitions: vec![
                        JavaDefinitionMetadata {
                            name: JavaName(quote!{TestInterface1}),
                            definition: JavaDefinitionMetadataKind::Interface(
                                JavaInterfaceMetadata { extends: vec![] },
                            ),
                        },
                        JavaDefinitionMetadata {
                            name: JavaName(quote!{TestInterface2}),
                            definition: JavaDefinitionMetadataKind::Interface(
                                JavaInterfaceMetadata {
                                    extends: vec![JavaName(quote!{TestInterface1})],
                                },
                            ),
                        },
                        JavaDefinitionMetadata {
                            name: JavaName(quote!{TestClass2}),
                            definition: JavaDefinitionMetadataKind::Class(JavaClassMetadata {
                                extends: None,
                                implements: vec![],
                            }),
                        },
                        JavaDefinitionMetadata {
                            name: JavaName(quote!{TestClass1}),
                            definition: JavaDefinitionMetadataKind::Class(JavaClassMetadata {
                                extends: Some(JavaName(quote!{TestClass2})),
                                implements: vec![
                                    JavaName(quote!{TestInterface1}),
                                    JavaName(quote!{TestInterface2}),
                                ],
                            }),
                        },
                    ],
                },
            }
        );
    }

    #[test]
    #[should_panic(expected = "Expected \"class\" or \"interface\"")]
    fn invalid_definition_kind() {
        let input = quote!{
            invalid 1
        };
        parse_java_definition(input);
    }

    #[test]
    #[should_panic(expected = "Expected a Java name")]
    fn too_few_tokens() {
        let input = quote!{
            class
        };
        parse_java_definition(input);
    }

    #[test]
    #[should_panic(expected = "Expected an identifier")]
    fn definition_name_not_identifier_after_dot() {
        let input = quote!{
            class a.1 {}
        };
        parse_java_definition(input);
    }

    #[test]
    #[should_panic(expected = "Expected a dot")]
    fn definition_name_no_dot_after_identifier() {
        let input = quote!{
            class a b {}
        };
        parse_java_definition(input);
    }

    #[test]
    #[should_panic(expected = "Expected a dot")]
    fn definition_name_not_dot_punctuation() {
        let input = quote!{
            class a,b {}
        };
        parse_java_definition(input);
    }

    #[test]
    #[should_panic(expected = "Expected braces")]
    fn metadata_not_group() {
        let input = quote!{
            metadata abc
        };
        parse_java_definition(input);
    }

    #[test]
    #[should_panic(expected = "Expected braces")]
    fn metadata_not_braces_group() {
        let input = quote!{
            metadata ()
        };
        parse_java_definition(input);
    }

    #[test]
    #[should_panic(expected = "Expected \"class\" or \"interface\"")]
    fn invalid_definition_metadata_kind() {
        let input = quote!{
            metadata {
                abc
            }
        };
        parse_java_definition(input);
    }
}

#[derive(Debug, Clone)]
struct ClassMethodGeneratorDefinition {
    name: Ident,
    java_name: Literal,
    return_type: TokenStream,
    argument_names: Vec<Ident>,
    argument_types: Vec<TokenStream>,
    public: TokenStream,
}

#[derive(Debug, Clone)]
struct ConstructorGeneratorDefinition {
    name: Ident,
    argument_names: Vec<Ident>,
    argument_types: Vec<TokenStream>,
    public: TokenStream,
}

#[derive(Debug, Clone)]
struct ClassGeneratorDefinition {
    class: Ident,
    public: TokenStream,
    super_class: TokenStream,
    transitive_extends: Vec<TokenStream>,
    implements: Vec<TokenStream>,
    signature: Literal,
    full_signature: Literal,
    constructors: Vec<ConstructorGeneratorDefinition>,
    methods: Vec<ClassMethodGeneratorDefinition>,
    static_methods: Vec<ClassMethodGeneratorDefinition>,
}

#[derive(Debug, Clone)]
struct InterfaceGeneratorDefinition {
    interface: Ident,
    public: TokenStream,
    extends: Vec<TokenStream>,
}

#[derive(Debug, Clone)]
enum GeneratorDefinition {
    Class(ClassGeneratorDefinition),
    Interface(InterfaceGeneratorDefinition),
}

impl PartialEq for GeneratorDefinition {
    fn eq(&self, other: &Self) -> bool {
        format!("{:?}", self) == format!("{:?}", other)
    }
}

impl Eq for GeneratorDefinition {}

#[derive(Debug, PartialEq, Eq, Clone)]
struct GeneratorData {
    definitions: Vec<GeneratorDefinition>,
}

fn populate_interface_extends_rec(
    interface_extends: &mut HashMap<JavaName, HashSet<JavaName>>,
    key: &JavaName,
) {
    let mut interfaces = interface_extends.get(key).unwrap().clone();
    // TODO: this will break in case of cycles.
    for interface in interfaces.iter() {
        populate_interface_extends_rec(interface_extends, interface)
    }
    for interface in interfaces.clone().iter() {
        interfaces.extend(interface_extends.get(interface).unwrap().iter().cloned());
    }
    *interface_extends.get_mut(key).unwrap() = interfaces;
}

fn populate_interface_extends(interface_extends: &mut HashMap<JavaName, HashSet<JavaName>>) {
    for key in interface_extends.keys().cloned().collect::<Vec<_>>() {
        populate_interface_extends_rec(interface_extends, &key);
    }
}

fn public_token(public: bool) -> TokenStream {
    if public {
        quote!{pub}
    } else {
        TokenStream::new()
    }
}

fn to_generator_method(method: JavaClassMethod) -> ClassMethodGeneratorDefinition {
    let JavaClassMethod {
        name,
        public,
        return_type,
        arguments,
        ..
    } = method;
    let public = public_token(public);
    let java_name = Literal::string(&name.to_string());
    ClassMethodGeneratorDefinition {
        name,
        java_name,
        public,
        return_type: return_type.as_rust_type(),
        argument_names: arguments
            .iter()
            .map(|argument| argument.name.clone())
            .collect(),
        argument_types: arguments
            .iter()
            .map(|argument| argument.data_type.clone().as_rust_type_reference())
            .collect(),
    }
}

fn to_generator_constructor(constructor: JavaConstructor) -> ConstructorGeneratorDefinition {
    let JavaConstructor {
        public, arguments, ..
    } = constructor;
    let public = public_token(public);
    let name = Ident::new("new", Span::call_site());
    ConstructorGeneratorDefinition {
        name,
        public,
        argument_names: arguments
            .iter()
            .map(|argument| argument.name.clone())
            .collect(),
        argument_types: arguments
            .iter()
            .map(|argument| argument.data_type.clone().as_rust_type_reference())
            .collect(),
    }
}

fn to_generator_data(definitions: JavaDefinitions) -> GeneratorData {
    let mut interface_extends = HashMap::new();
    definitions
        .definitions
        .clone()
        .into_iter()
        .filter(|definition| match definition.definition {
            JavaDefinitionKind::Interface(_) => true,
            _ => false,
        })
        .for_each(|definition| {
            let JavaDefinition {
                name, definition, ..
            } = definition;
            match definition {
                JavaDefinitionKind::Interface(interface) => {
                    let JavaInterface { extends, .. } = interface;
                    let all_extends = interface_extends.entry(name).or_insert(HashSet::new());
                    extends.into_iter().for_each(|extends_name| {
                        all_extends.insert(extends_name);
                    });
                }
                _ => unreachable!(),
            }
        });
    definitions
        .metadata
        .definitions
        .clone()
        .into_iter()
        .filter(|definition| match definition.definition {
            JavaDefinitionMetadataKind::Interface(_) => true,
            _ => false,
        })
        .for_each(|definition| {
            let JavaDefinitionMetadata {
                name, definition, ..
            } = definition;
            match definition {
                JavaDefinitionMetadataKind::Interface(interface) => {
                    let JavaInterfaceMetadata { extends, .. } = interface;
                    let all_extends = interface_extends.entry(name).or_insert(HashSet::new());
                    extends.into_iter().for_each(|extends_name| {
                        all_extends.insert(extends_name);
                    });
                }
                _ => unreachable!(),
            }
        });
    populate_interface_extends(&mut interface_extends);
    let mut extends_map = HashMap::new();
    definitions
        .definitions
        .clone()
        .into_iter()
        .filter(|definition| match definition.definition {
            JavaDefinitionKind::Class(_) => true,
            _ => false,
        })
        .for_each(|definition| {
            let JavaDefinition {
                name, definition, ..
            } = definition;
            match definition {
                JavaDefinitionKind::Class(class) => {
                    let JavaClass { extends, .. } = class;
                    extends_map.insert(name, extends.unwrap_or(JavaName(quote!{java lang Object})));
                }
                _ => unreachable!(),
            }
        });
    definitions
        .metadata
        .definitions
        .clone()
        .into_iter()
        .filter(|definition| match definition.definition {
            JavaDefinitionMetadataKind::Class(_) => true,
            _ => false,
        })
        .for_each(|definition| {
            let JavaDefinitionMetadata {
                name, definition, ..
            } = definition;
            match definition {
                JavaDefinitionMetadataKind::Class(class) => {
                    let JavaClassMetadata { extends, .. } = class;
                    extends_map.insert(name, extends.unwrap_or(JavaName(quote!{java lang Object})));
                }
                _ => unreachable!(),
            }
        });
    GeneratorData {
        definitions: definitions
            .definitions
            .into_iter()
            .map(|definition| {
                let JavaDefinition {
                    name,
                    public,
                    definition,
                    ..
                } = definition;
                let definition_name = name.clone().name();
                let public = public_token(public);
                match definition {
                    JavaDefinitionKind::Class(class) => {
                        let JavaClass {
                            extends,
                            implements,
                            constructors,
                            methods,
                            ..
                        } = class;
                        let mut transitive_extends = vec![];
                        let mut current = name.clone();
                        loop {
                            let super_class = extends_map.get(&current);
                            if super_class.is_none() {
                                break;
                            }
                            let super_class = super_class.unwrap();
                            transitive_extends.push(super_class.clone().with_double_colons());
                            current = super_class.clone();
                        }
                        let string_signature = name.with_slashes();
                        let signature = Literal::string(&string_signature);
                        let full_signature = Literal::string(&format!("L{};", string_signature));
                        let super_class = extends
                            .map(|name| name.with_double_colons())
                            .unwrap_or(quote!{::java::lang::Object});
                        let implements = implements
                            .iter()
                            .flat_map(|name| interface_extends.get(&name).unwrap().iter())
                            .chain(implements.iter())
                            .cloned()
                            .collect::<HashSet<_>>();
                        let mut implements = implements
                            .into_iter()
                            .map(|name| name.with_double_colons())
                            .collect::<Vec<_>>();
                        implements.sort_by(|left, right| left.to_string().cmp(&right.to_string()));
                        let static_methods = methods
                            .iter()
                            .filter(|method| method.is_static)
                            .cloned()
                            .map(to_generator_method)
                            .collect();
                        let methods = methods
                            .iter()
                            .filter(|method| !method.is_static)
                            .cloned()
                            .map(to_generator_method)
                            .collect();
                        let constructors = constructors
                            .into_iter()
                            .map(to_generator_constructor)
                            .collect();
                        GeneratorDefinition::Class(ClassGeneratorDefinition {
                            class: definition_name,
                            public,
                            super_class,
                            transitive_extends,
                            implements,
                            signature,
                            full_signature,
                            constructors,
                            methods,
                            static_methods,
                        })
                    }
                    JavaDefinitionKind::Interface(interface) => {
                        let JavaInterface { extends, .. } = interface;
                        GeneratorDefinition::Interface(InterfaceGeneratorDefinition {
                            interface: definition_name,
                            public,
                            extends: extends
                                .into_iter()
                                .map(|name| name.with_double_colons())
                                .collect(),
                        })
                    }
                }
            })
            .collect(),
    }
}

#[cfg(test)]
mod to_generator_data_tests {
    use super::*;

    #[test]
    fn empty() {
        assert_eq!(
            to_generator_data(JavaDefinitions {
                definitions: vec![],
                metadata: Metadata {
                    definitions: vec![],
                },
            }),
            GeneratorData {
                definitions: vec![],
            }
        );
    }

    #[test]
    fn metadata_only() {
        assert_eq!(
            to_generator_data(JavaDefinitions {
                definitions: vec![],
                metadata: Metadata {
                    definitions: vec![
                        JavaDefinitionMetadata {
                            name: JavaName(quote!{c d test1}),
                            definition: JavaDefinitionMetadataKind::Interface(
                                JavaInterfaceMetadata { extends: vec![] },
                            ),
                        },
                        JavaDefinitionMetadata {
                            name: JavaName(quote!{a b test2}),
                            definition: JavaDefinitionMetadataKind::Class(JavaClassMetadata {
                                extends: None,
                                implements: vec![JavaName(quote!{c d test1})],
                            }),
                        },
                    ],
                },
            }),
            GeneratorData {
                definitions: vec![],
            }
        );
    }

    #[test]
    fn one_class() {
        assert_eq!(
            to_generator_data(JavaDefinitions {
                definitions: vec![JavaDefinition {
                    name: JavaName(quote!{a b test1}),
                    public: false,
                    definition: JavaDefinitionKind::Class(JavaClass {
                        extends: Some(JavaName(quote!{c d test2})),
                        implements: vec![],
                        methods: vec![],
                        constructors: vec![],
                    }),
                }],
                metadata: Metadata {
                    definitions: vec![],
                },
            }),
            GeneratorData {
                definitions: vec![GeneratorDefinition::Class(ClassGeneratorDefinition {
                    class: Ident::new("test1", Span::call_site()),
                    public: TokenStream::new(),
                    super_class: quote!{::c::d::test2},
                    transitive_extends: vec![quote!{::c::d::test2}],
                    implements: vec![],
                    signature: Literal::string("a/b/test1"),
                    full_signature: Literal::string("La/b/test1;"),
                    methods: vec![],
                    static_methods: vec![],
                    constructors: vec![],
                })],
            }
        );
    }

    #[test]
    fn one_class_no_extends() {
        assert_eq!(
            to_generator_data(JavaDefinitions {
                definitions: vec![JavaDefinition {
                    name: JavaName(quote!{a b test1}),
                    public: false,
                    definition: JavaDefinitionKind::Class(JavaClass {
                        extends: None,
                        implements: vec![],
                        methods: vec![],
                        constructors: vec![],
                    }),
                }],
                metadata: Metadata {
                    definitions: vec![],
                },
            }),
            GeneratorData {
                definitions: vec![GeneratorDefinition::Class(ClassGeneratorDefinition {
                    class: Ident::new("test1", Span::call_site()),
                    public: TokenStream::new(),
                    super_class: quote!{::java::lang::Object},
                    transitive_extends: vec![quote!{::java::lang::Object}],
                    implements: vec![],
                    signature: Literal::string("a/b/test1"),
                    full_signature: Literal::string("La/b/test1;"),
                    methods: vec![],
                    static_methods: vec![],
                    constructors: vec![],
                })],
            }
        );
    }

    #[test]
    fn one_class_extends_recursive() {
        assert_eq!(
            to_generator_data(JavaDefinitions {
                definitions: vec![
                    JavaDefinition {
                        name: JavaName(quote!{c d test2}),
                        public: false,
                        definition: JavaDefinitionKind::Class(JavaClass {
                            extends: Some(JavaName(quote!{e f test3})),
                            implements: vec![],
                            methods: vec![],
                            constructors: vec![],
                        }),
                    },
                    JavaDefinition {
                        name: JavaName(quote!{a b test1}),
                        public: false,
                        definition: JavaDefinitionKind::Class(JavaClass {
                            extends: Some(JavaName(quote!{c d test2})),
                            implements: vec![],
                            methods: vec![],
                            constructors: vec![],
                        }),
                    },
                ],
                metadata: Metadata {
                    definitions: vec![
                        JavaDefinitionMetadata {
                            name: JavaName(quote!{e f test4}),
                            definition: JavaDefinitionMetadataKind::Class(JavaClassMetadata {
                                extends: None,
                                implements: vec![],
                            }),
                        },
                        JavaDefinitionMetadata {
                            name: JavaName(quote!{e f test3}),
                            definition: JavaDefinitionMetadataKind::Class(JavaClassMetadata {
                                extends: Some(JavaName(quote!{e f test4})),
                                implements: vec![],
                            }),
                        },
                    ],
                },
            }),
            GeneratorData {
                definitions: vec![
                    GeneratorDefinition::Class(ClassGeneratorDefinition {
                        class: Ident::new("test2", Span::call_site()),
                        public: TokenStream::new(),
                        super_class: quote!{::e::f::test3},
                        transitive_extends: vec![
                            quote!{::e::f::test3},
                            quote!{::e::f::test4},
                            quote!{::java::lang::Object},
                        ],
                        implements: vec![],
                        signature: Literal::string("c/d/test2"),
                        full_signature: Literal::string("Lc/d/test2;"),
                        methods: vec![],
                        static_methods: vec![],
                        constructors: vec![],
                    }),
                    GeneratorDefinition::Class(ClassGeneratorDefinition {
                        class: Ident::new("test1", Span::call_site()),
                        public: TokenStream::new(),
                        super_class: quote!{::c::d::test2},
                        transitive_extends: vec![
                            quote!{::c::d::test2},
                            quote!{::e::f::test3},
                            quote!{::e::f::test4},
                            quote!{::java::lang::Object},
                        ],
                        implements: vec![],
                        signature: Literal::string("a/b/test1"),
                        full_signature: Literal::string("La/b/test1;"),
                        methods: vec![],
                        static_methods: vec![],
                        constructors: vec![],
                    }),
                ],
            }
        );
    }

    #[test]
    fn one_class_implements() {
        assert_eq!(
            to_generator_data(JavaDefinitions {
                definitions: vec![
                    JavaDefinition {
                        name: JavaName(quote!{e f test4}),
                        public: false,
                        definition: JavaDefinitionKind::Interface(JavaInterface {
                            extends: vec![],
                        }),
                    },
                    JavaDefinition {
                        name: JavaName(quote!{a b test1}),
                        public: false,
                        definition: JavaDefinitionKind::Class(JavaClass {
                            extends: None,
                            implements: vec![
                                JavaName(quote!{e f test3}),
                                JavaName(quote!{e f test4}),
                            ],
                            methods: vec![],
                            constructors: vec![],
                        }),
                    },
                ],
                metadata: Metadata {
                    definitions: vec![JavaDefinitionMetadata {
                        name: JavaName(quote!{e f test3}),
                        definition: JavaDefinitionMetadataKind::Interface(JavaInterfaceMetadata {
                            extends: vec![],
                        }),
                    }],
                },
            }),
            GeneratorData {
                definitions: vec![
                    GeneratorDefinition::Interface(InterfaceGeneratorDefinition {
                        interface: Ident::new("test4", Span::call_site()),
                        public: TokenStream::new(),
                        extends: vec![],
                    }),
                    GeneratorDefinition::Class(ClassGeneratorDefinition {
                        class: Ident::new("test1", Span::call_site()),
                        public: TokenStream::new(),
                        super_class: quote!{::java::lang::Object},
                        transitive_extends: vec![quote!{::java::lang::Object}],
                        implements: vec![quote!{::e::f::test3}, quote!{::e::f::test4}],
                        signature: Literal::string("a/b/test1"),
                        full_signature: Literal::string("La/b/test1;"),
                        methods: vec![],
                        static_methods: vec![],
                        constructors: vec![],
                    }),
                ],
            }
        );
    }

    #[test]
    fn one_class_implements_recursive() {
        assert_eq!(
            to_generator_data(JavaDefinitions {
                definitions: vec![
                    JavaDefinition {
                        name: JavaName(quote!{e f test3}),
                        public: false,
                        definition: JavaDefinitionKind::Interface(JavaInterface {
                            extends: vec![JavaName(quote!{e f test4})],
                        }),
                    },
                    JavaDefinition {
                        name: JavaName(quote!{a b test1}),
                        public: false,
                        definition: JavaDefinitionKind::Class(JavaClass {
                            extends: None,
                            implements: vec![JavaName(quote!{e f test3})],
                            methods: vec![],
                            constructors: vec![],
                        }),
                    },
                ],
                metadata: Metadata {
                    definitions: vec![
                        JavaDefinitionMetadata {
                            name: JavaName(quote!{g h test5}),
                            definition: JavaDefinitionMetadataKind::Interface(
                                JavaInterfaceMetadata { extends: vec![] },
                            ),
                        },
                        JavaDefinitionMetadata {
                            name: JavaName(quote!{e f test4}),
                            definition: JavaDefinitionMetadataKind::Interface(
                                JavaInterfaceMetadata {
                                    extends: vec![JavaName(quote!{g h test5})],
                                },
                            ),
                        },
                    ],
                },
            }),
            GeneratorData {
                definitions: vec![
                    GeneratorDefinition::Interface(InterfaceGeneratorDefinition {
                        interface: Ident::new("test3", Span::call_site()),
                        public: TokenStream::new(),
                        extends: vec![quote!{::e::f::test4}],
                    }),
                    GeneratorDefinition::Class(ClassGeneratorDefinition {
                        class: Ident::new("test1", Span::call_site()),
                        public: TokenStream::new(),
                        super_class: quote!{::java::lang::Object},
                        transitive_extends: vec![quote!{::java::lang::Object}],
                        implements: vec![
                            quote!{::e::f::test3},
                            quote!{::e::f::test4},
                            quote!{::g::h::test5},
                        ],
                        signature: Literal::string("a/b/test1"),
                        full_signature: Literal::string("La/b/test1;"),
                        methods: vec![],
                        static_methods: vec![],
                        constructors: vec![],
                    }),
                ],
            }
        );
    }

    #[test]
    fn one_class_implements_recursive_duplicated() {
        assert_eq!(
            to_generator_data(JavaDefinitions {
                definitions: vec![
                    JavaDefinition {
                        name: JavaName(quote!{g h test4}),
                        public: false,
                        definition: JavaDefinitionKind::Interface(JavaInterface {
                            extends: vec![],
                        }),
                    },
                    JavaDefinition {
                        name: JavaName(quote!{e f test3}),
                        public: false,
                        definition: JavaDefinitionKind::Interface(JavaInterface {
                            extends: vec![JavaName(quote!{g h test4})],
                        }),
                    },
                    JavaDefinition {
                        name: JavaName(quote!{a b test1}),
                        public: false,
                        definition: JavaDefinitionKind::Class(JavaClass {
                            extends: None,
                            implements: vec![
                                JavaName(quote!{e f test3}),
                                JavaName(quote!{g h test4}),
                            ],
                            methods: vec![],
                            constructors: vec![],
                        }),
                    },
                ],
                metadata: Metadata {
                    definitions: vec![],
                },
            }),
            GeneratorData {
                definitions: vec![
                    GeneratorDefinition::Interface(InterfaceGeneratorDefinition {
                        interface: Ident::new("test4", Span::call_site()),
                        public: TokenStream::new(),
                        extends: vec![],
                    }),
                    GeneratorDefinition::Interface(InterfaceGeneratorDefinition {
                        interface: Ident::new("test3", Span::call_site()),
                        public: TokenStream::new(),
                        extends: vec![quote!{::g::h::test4}],
                    }),
                    GeneratorDefinition::Class(ClassGeneratorDefinition {
                        class: Ident::new("test1", Span::call_site()),
                        public: TokenStream::new(),
                        super_class: quote!{::java::lang::Object},
                        transitive_extends: vec![quote!{::java::lang::Object}],
                        implements: vec![quote!{::e::f::test3}, quote!{::g::h::test4}],
                        signature: Literal::string("a/b/test1"),
                        full_signature: Literal::string("La/b/test1;"),
                        methods: vec![],
                        static_methods: vec![],
                        constructors: vec![],
                    }),
                ],
            }
        );
    }

    #[test]
    fn one_class_public() {
        assert_eq!(
            to_generator_data(JavaDefinitions {
                definitions: vec![JavaDefinition {
                    name: JavaName(quote!{a b test1}),
                    public: true,
                    definition: JavaDefinitionKind::Class(JavaClass {
                        extends: None,
                        implements: vec![],
                        methods: vec![],
                        constructors: vec![],
                    }),
                }],
                metadata: Metadata {
                    definitions: vec![],
                },
            }),
            GeneratorData {
                definitions: vec![GeneratorDefinition::Class(ClassGeneratorDefinition {
                    class: Ident::new("test1", Span::call_site()),
                    public: quote!{pub},
                    super_class: quote!{::java::lang::Object},
                    transitive_extends: vec![quote!{::java::lang::Object}],
                    implements: vec![],
                    signature: Literal::string("a/b/test1"),
                    full_signature: Literal::string("La/b/test1;"),
                    methods: vec![],
                    static_methods: vec![],
                    constructors: vec![],
                })],
            }
        );
    }

    #[test]
    fn one_interface() {
        assert_eq!(
            to_generator_data(JavaDefinitions {
                definitions: vec![JavaDefinition {
                    name: JavaName(quote!{a b test1}),
                    public: false,
                    definition: JavaDefinitionKind::Interface(JavaInterface { extends: vec![] }),
                }],
                metadata: Metadata {
                    definitions: vec![],
                },
            }),
            GeneratorData {
                definitions: vec![GeneratorDefinition::Interface(
                    InterfaceGeneratorDefinition {
                        interface: Ident::new("test1", Span::call_site()),
                        public: TokenStream::new(),
                        extends: vec![],
                    },
                )],
            }
        );
    }

    #[test]
    fn one_interface_extends() {
        assert_eq!(
            to_generator_data(JavaDefinitions {
                definitions: vec![
                    JavaDefinition {
                        name: JavaName(quote!{e f test3}),
                        public: false,
                        definition: JavaDefinitionKind::Interface(JavaInterface {
                            extends: vec![],
                        }),
                    },
                    JavaDefinition {
                        name: JavaName(quote!{a b test1}),
                        public: false,
                        definition: JavaDefinitionKind::Interface(JavaInterface {
                            extends: vec![JavaName(quote!{c d test2}), JavaName(quote!{e f test3})],
                        }),
                    },
                ],
                metadata: Metadata {
                    definitions: vec![
                        JavaDefinitionMetadata {
                            name: JavaName(quote!{c d test4}),
                            definition: JavaDefinitionMetadataKind::Interface(
                                JavaInterfaceMetadata { extends: vec![] },
                            ),
                        },
                        JavaDefinitionMetadata {
                            name: JavaName(quote!{c d test2}),
                            definition: JavaDefinitionMetadataKind::Interface(
                                JavaInterfaceMetadata {
                                    extends: vec![JavaName(quote!{c d test4})],
                                },
                            ),
                        },
                    ],
                },
            }),
            GeneratorData {
                definitions: vec![
                    GeneratorDefinition::Interface(InterfaceGeneratorDefinition {
                        interface: Ident::new("test3", Span::call_site()),
                        public: TokenStream::new(),
                        extends: vec![],
                    }),
                    GeneratorDefinition::Interface(InterfaceGeneratorDefinition {
                        interface: Ident::new("test1", Span::call_site()),
                        public: TokenStream::new(),
                        extends: vec![quote!{::c::d::test2}, quote!{::e::f::test3}],
                    }),
                ],
            }
        );
    }

    #[test]
    fn one_interface_public() {
        assert_eq!(
            to_generator_data(JavaDefinitions {
                definitions: vec![JavaDefinition {
                    name: JavaName(quote!{a b test1}),
                    public: true,
                    definition: JavaDefinitionKind::Interface(JavaInterface { extends: vec![] }),
                }],
                metadata: Metadata {
                    definitions: vec![],
                },
            }),
            GeneratorData {
                definitions: vec![GeneratorDefinition::Interface(
                    InterfaceGeneratorDefinition {
                        interface: Ident::new("test1", Span::call_site()),
                        public: quote!{pub},
                        extends: vec![],
                    },
                )],
            }
        );
    }

    #[test]
    fn multiple() {
        assert_eq!(
            to_generator_data(JavaDefinitions {
                definitions: vec![
                    JavaDefinition {
                        name: JavaName(quote!{e f test_if1}),
                        public: false,
                        definition: JavaDefinitionKind::Interface(JavaInterface {
                            extends: vec![],
                        }),
                    },
                    JavaDefinition {
                        name: JavaName(quote!{e f test_if2}),
                        public: false,
                        definition: JavaDefinitionKind::Interface(JavaInterface {
                            extends: vec![],
                        }),
                    },
                    JavaDefinition {
                        name: JavaName(quote!{a b test1}),
                        public: false,
                        definition: JavaDefinitionKind::Class(JavaClass {
                            extends: None,
                            implements: vec![],
                            methods: vec![],
                            constructors: vec![],
                        }),
                    },
                    JavaDefinition {
                        name: JavaName(quote!{test2}),
                        public: false,
                        definition: JavaDefinitionKind::Class(JavaClass {
                            extends: None,
                            implements: vec![],
                            methods: vec![],
                            constructors: vec![],
                        }),
                    },
                ],
                metadata: Metadata {
                    definitions: vec![],
                },
            }),
            GeneratorData {
                definitions: vec![
                    GeneratorDefinition::Interface(InterfaceGeneratorDefinition {
                        interface: Ident::new("test_if1", Span::call_site()),
                        public: TokenStream::new(),
                        extends: vec![],
                    }),
                    GeneratorDefinition::Interface(InterfaceGeneratorDefinition {
                        interface: Ident::new("test_if2", Span::call_site()),
                        public: TokenStream::new(),
                        extends: vec![],
                    }),
                    GeneratorDefinition::Class(ClassGeneratorDefinition {
                        class: Ident::new("test1", Span::call_site()),
                        public: TokenStream::new(),
                        super_class: quote!{::java::lang::Object},
                        transitive_extends: vec![quote!{::java::lang::Object}],
                        implements: vec![],
                        signature: Literal::string("a/b/test1"),
                        full_signature: Literal::string("La/b/test1;"),
                        methods: vec![],
                        static_methods: vec![],
                        constructors: vec![],
                    }),
                    GeneratorDefinition::Class(ClassGeneratorDefinition {
                        class: Ident::new("test2", Span::call_site()),
                        public: TokenStream::new(),
                        super_class: quote!{::java::lang::Object},
                        transitive_extends: vec![quote!{::java::lang::Object}],
                        implements: vec![],
                        signature: Literal::string("test2"),
                        full_signature: Literal::string("Ltest2;"),
                        methods: vec![],
                        static_methods: vec![],
                        constructors: vec![],
                    }),
                ],
            }
        );
    }
}

fn generate(data: GeneratorData) -> TokenStream {
    let mut tokens = TokenStream::new();
    for definition in data.definitions {
        tokens.extend(generate_definition(definition));
    }
    tokens
}

fn generate_definition(definition: GeneratorDefinition) -> TokenStream {
    match definition {
        GeneratorDefinition::Class(class) => generate_class_definition(class),
        GeneratorDefinition::Interface(interface) => generate_interface_definition(interface),
    }
}

fn generate_class_method(method: ClassMethodGeneratorDefinition) -> TokenStream {
    let ClassMethodGeneratorDefinition {
        name,
        java_name,
        return_type,
        public,
        argument_names,
        argument_types,
    } = method;
    let argument_names_1 = argument_names.clone();
    let argument_types_1 = argument_types.clone();
    quote!{
        #public fn #name(
            &self,
            #(#argument_names: #argument_types,)*
            token: &::rust_jni::NoException<'a>,
        ) -> ::rust_jni::JavaResult<'a, #return_type> {
            // Safe because the method name and arguments are correct.
            unsafe {
                ::rust_jni::__generator::call_method::<_, _, _,
                    fn(#(#argument_types_1,)*) -> #return_type
                >
                (
                    self,
                    #java_name,
                    (#(#argument_names_1,)*),
                    token,
                )
            }
        }
    }
}

fn generate_static_class_method(method: ClassMethodGeneratorDefinition) -> TokenStream {
    let ClassMethodGeneratorDefinition {
        name,
        java_name,
        return_type,
        public,
        argument_names,
        argument_types,
    } = method;
    let argument_names_1 = argument_names.clone();
    let argument_types_1 = argument_types.clone();
    quote!{
        #public fn #name(
            env: &'a ::rust_jni::JniEnv<'a>,
            #(#argument_names: #argument_types,)*
            token: &::rust_jni::NoException<'a>,
        ) -> ::rust_jni::JavaResult<'a, #return_type> {
            // Safe because the method name and arguments are correct.
            unsafe {
                ::rust_jni::__generator::call_static_method::<Self, _, _,
                    fn(#(#argument_types_1,)*) -> #return_type
                >
                (
                    env,
                    #java_name,
                    (#(#argument_names_1,)*),
                    token,
                )
            }
        }
    }
}

fn generate_constructor(method: ConstructorGeneratorDefinition) -> TokenStream {
    let ConstructorGeneratorDefinition {
        name,
        public,
        argument_names,
        argument_types,
    } = method;
    let argument_names_1 = argument_names.clone();
    let argument_types_1 = argument_types.clone();
    quote!{
        #public fn #name(
            env: &'a ::rust_jni::JniEnv<'a>,
            #(#argument_names: #argument_types,)*
            token: &::rust_jni::NoException<'a>,
        ) -> ::rust_jni::JavaResult<'a, Self> {
            // Safe because the method name and arguments are correct.
            unsafe {
                ::rust_jni::__generator::call_constructor::<Self, _, fn(#(#argument_types_1,)*)>
                (
                    env,
                    (#(#argument_names_1,)*),
                    token,
                )
            }
        }
    }
}

fn generate_class_definition(definition: ClassGeneratorDefinition) -> TokenStream {
    let ClassGeneratorDefinition {
        class,
        public,
        super_class,
        transitive_extends,
        implements,
        signature,
        full_signature,
        constructors,
        methods,
        static_methods,
        ..
    } = definition;
    let multiplied_class = iter::repeat(class.clone());
    let multiplied_class_1 = multiplied_class.clone();
    let transitive_extends_1 = transitive_extends.clone();
    let methods = methods
        .into_iter()
        .map(generate_class_method)
        .collect::<Vec<_>>();
    let static_methods = static_methods
        .into_iter()
        .map(generate_static_class_method)
        .collect::<Vec<_>>();
    let constructors = constructors
        .into_iter()
        .map(generate_constructor)
        .collect::<Vec<_>>();
    quote! {
        #[derive(Debug)]
        #public struct #class<'env> {
            object: #super_class<'env>,
        }

        impl<'a> ::rust_jni::JavaType for #class<'a> {
            #[doc(hidden)]
            type __JniType = <::rust_jni::java::lang::Object<'a> as ::rust_jni::JavaType>::__JniType;

            #[doc(hidden)]
            fn __signature() -> &'static str {
                #full_signature
            }
        }

        impl<'a> ::rust_jni::__generator::ToJni for #class<'a> {
            unsafe fn __to_jni(&self) -> Self::__JniType {
                self.raw_object()
            }
        }

        impl<'a> ::rust_jni::__generator::FromJni<'a> for #class<'a> {
            unsafe fn __from_jni(env: &'a ::rust_jni::JniEnv<'a>, value: Self::__JniType) -> Self {
                Self {
                    object: <#super_class as ::rust_jni::__generator::FromJni<'a>>::__from_jni(env, value),
                }
            }
        }

        impl<'a> ::rust_jni::Cast<'a, #class<'a>> for #class<'a> {
            #[doc(hidden)]
            fn cast<'b>(&'b self) -> &'b #class<'a> {
                self
            }
        }

        #(
            impl<'a> ::rust_jni::Cast<'a, #transitive_extends<'a>> for #multiplied_class_1<'a> {
                #[doc(hidden)]
                fn cast<'b>(&'b self) -> &'b #transitive_extends_1<'a> {
                    self
                }
            }
        )*

        impl<'a> ::std::ops::Deref for #class<'a> {
            type Target = #super_class<'a>;

            fn deref(&self) -> &Self::Target {
                &self.object
            }
        }

        impl<'a> #class<'a> {
            pub fn get_class(env: &'a ::rust_jni::JniEnv<'a>, token: &::rust_jni::NoException<'a>)
                -> ::rust_jni::JavaResult<'a, ::rust_jni::java::lang::Class<'a>> {
                ::rust_jni::java::lang::Class::find(env, #signature, token)
            }

            pub fn clone(&self, token: &::rust_jni::NoException<'a>) -> ::rust_jni::JavaResult<'a, Self>
            where
                Self: Sized,
            {
                self.object
                    .clone(token)
                    .map(|object| Self { object })
            }

            pub fn to_string(&self, token: &::rust_jni::NoException<'a>)
                -> ::rust_jni::JavaResult<'a, ::rust_jni::java::lang::String<'a>> {
                self.object.to_string(token)
            }

            #(
                #constructors
            )*

            #(
                #methods
            )*

            #(
                #static_methods
            )*
        }

        impl<'a> ::std::fmt::Display for #class<'a> {
            fn fmt(&self, formatter: &mut ::std::fmt::Formatter) -> ::std::fmt::Result {
                self.object.fmt(formatter)
            }
        }

        impl<'a, T> PartialEq<T> for #class<'a> where T: ::rust_jni::Cast<'a, ::rust_jni::java::lang::Object<'a>> {
            fn eq(&self, other: &T) -> bool {
                self.object.eq(other)
            }
        }

        impl<'a> Eq for #class<'a> {}

        #(
            impl<'a> #implements for #multiplied_class<'a> {
            }
        )*
    }
}

fn generate_interface_definition(definition: InterfaceGeneratorDefinition) -> TokenStream {
    let InterfaceGeneratorDefinition {
        interface,
        public,
        extends,
        ..
    } = definition;
    let extends = if extends.is_empty() {
        TokenStream::new()
    } else {
        quote!{: #(#extends)+*}
    };
    quote! {
        #public trait #interface #extends {
        }
    }
}

#[cfg(test)]
mod generate_tests {
    use super::*;

    #[test]
    fn empty() {
        let input = GeneratorData {
            definitions: vec![],
        };
        let expected = quote!{};
        assert_tokens_equals(generate(input), expected);
    }

    #[test]
    fn one_class() {
        let input = GeneratorData {
            definitions: vec![GeneratorDefinition::Class(ClassGeneratorDefinition {
                class: Ident::new("test1", Span::call_site()),
                public: quote!{test_public},
                super_class: quote!{c::d::test2},
                transitive_extends: vec![quote!{c::d::test2}],
                implements: vec![],
                signature: Literal::string("test/sign1"),
                full_signature: Literal::string("test/signature1"),
                methods: vec![],
                static_methods: vec![],
                constructors: vec![],
            })],
        };
        let expected = quote!{
            #[derive(Debug)]
            test_public struct test1<'env> {
                object: c::d::test2<'env>,
            }

            impl<'a> ::rust_jni::JavaType for test1<'a> {
                #[doc(hidden)]
                type __JniType = <::rust_jni::java::lang::Object<'a> as ::rust_jni::JavaType>::__JniType;

                #[doc(hidden)]
                fn __signature() -> &'static str {
                    "test/signature1"
                }
            }

            impl<'a> ::rust_jni::__generator::ToJni for test1<'a> {
                unsafe fn __to_jni(&self) -> Self::__JniType {
                    self.raw_object()
                }
            }

            impl<'a> ::rust_jni::__generator::FromJni<'a> for test1<'a> {
                unsafe fn __from_jni(env: &'a ::rust_jni::JniEnv<'a>, value: Self::__JniType) -> Self {
                    Self {
                        object: <c::d::test2 as ::rust_jni::__generator::FromJni<'a>>::__from_jni(env, value),
                    }
                }
            }

            impl<'a> ::rust_jni::Cast<'a, test1<'a>> for test1<'a> {
                #[doc(hidden)]
                fn cast<'b>(&'b self) -> &'b test1<'a> {
                    self
                }
            }

            impl<'a> ::rust_jni::Cast<'a, c::d::test2<'a>> for test1<'a> {
                #[doc(hidden)]
                fn cast<'b>(&'b self) -> &'b c::d::test2<'a> {
                    self
                }
            }

            impl<'a> ::std::ops::Deref for test1<'a> {
                type Target = c::d::test2<'a>;

                fn deref(&self) -> &Self::Target {
                    &self.object
                }
            }

            impl<'a> test1<'a> {
                pub fn get_class(env: &'a ::rust_jni::JniEnv<'a>, token: &::rust_jni::NoException<'a>)
                    -> ::rust_jni::JavaResult<'a, ::rust_jni::java::lang::Class<'a>> {
                    ::rust_jni::java::lang::Class::find(env, "test/sign1", token)
                }

                pub fn clone(&self, token: &::rust_jni::NoException<'a>) -> ::rust_jni::JavaResult<'a, Self>
                where
                    Self: Sized,
                {
                    self.object
                        .clone(token)
                        .map(|object| Self { object })
                }

                pub fn to_string(&self, token: &::rust_jni::NoException<'a>)
                    -> ::rust_jni::JavaResult<'a, ::rust_jni::java::lang::String<'a>> {
                    self.object.to_string(token)
                }
            }

            impl<'a> ::std::fmt::Display for test1<'a> {
                fn fmt(&self, formatter: &mut ::std::fmt::Formatter) -> ::std::fmt::Result {
                    self.object.fmt(formatter)
                }
            }

            impl<'a, T> PartialEq<T> for test1<'a> where T: ::rust_jni::Cast<'a, ::rust_jni::java::lang::Object<'a>> {
                fn eq(&self, other: &T) -> bool {
                    self.object.eq(other)
                }
            }

            impl<'a> Eq for test1<'a> {}
        };
        assert_tokens_equals(generate(input), expected);
    }

    #[test]
    fn one_class_implements() {
        let input = GeneratorData {
            definitions: vec![GeneratorDefinition::Class(ClassGeneratorDefinition {
                class: Ident::new("test1", Span::call_site()),
                public: quote!{test_public},
                super_class: quote!{c::d::test2},
                transitive_extends: vec![quote!{c::d::test2}],
                implements: vec![quote!{e::f::test3}, quote!{e::f::test4}],
                signature: Literal::string("test/sign1"),
                full_signature: Literal::string("test/signature1"),
                methods: vec![],
                static_methods: vec![],
                constructors: vec![],
            })],
        };
        let expected = quote!{
            #[derive(Debug)]
            test_public struct test1<'env> {
                object: c::d::test2<'env>,
            }

            impl<'a> ::rust_jni::JavaType for test1<'a> {
                #[doc(hidden)]
                type __JniType = <::rust_jni::java::lang::Object<'a> as ::rust_jni::JavaType>::__JniType;

                #[doc(hidden)]
                fn __signature() -> &'static str {
                    "test/signature1"
                }
            }

            impl<'a> ::rust_jni::__generator::ToJni for test1<'a> {
                unsafe fn __to_jni(&self) -> Self::__JniType {
                    self.raw_object()
                }
            }

            impl<'a> ::rust_jni::__generator::FromJni<'a> for test1<'a> {
                unsafe fn __from_jni(env: &'a ::rust_jni::JniEnv<'a>, value: Self::__JniType) -> Self {
                    Self {
                        object: <c::d::test2 as ::rust_jni::__generator::FromJni<'a>>::__from_jni(env, value),
                    }
                }
            }

            impl<'a> ::rust_jni::Cast<'a, test1<'a>> for test1<'a> {
                #[doc(hidden)]
                fn cast<'b>(&'b self) -> &'b test1<'a> {
                    self
                }
            }

            impl<'a> ::rust_jni::Cast<'a, c::d::test2<'a>> for test1<'a> {
                #[doc(hidden)]
                fn cast<'b>(&'b self) -> &'b c::d::test2<'a> {
                    self
                }
            }

            impl<'a> ::std::ops::Deref for test1<'a> {
                type Target = c::d::test2<'a>;

                fn deref(&self) -> &Self::Target {
                    &self.object
                }
            }

            impl<'a> test1<'a> {
                pub fn get_class(env: &'a ::rust_jni::JniEnv<'a>, token: &::rust_jni::NoException<'a>)
                    -> ::rust_jni::JavaResult<'a, ::rust_jni::java::lang::Class<'a>> {
                    ::rust_jni::java::lang::Class::find(env, "test/sign1", token)
                }

                pub fn clone(&self, token: &::rust_jni::NoException<'a>) -> ::rust_jni::JavaResult<'a, Self>
                where
                    Self: Sized,
                {
                    self.object
                        .clone(token)
                        .map(|object| Self { object })
                }

                pub fn to_string(&self, token: &::rust_jni::NoException<'a>)
                    -> ::rust_jni::JavaResult<'a, ::rust_jni::java::lang::String<'a>> {
                    self.object.to_string(token)
                }
            }

            impl<'a> ::std::fmt::Display for test1<'a> {
                fn fmt(&self, formatter: &mut ::std::fmt::Formatter) -> ::std::fmt::Result {
                    self.object.fmt(formatter)
                }
            }

            impl<'a, T> PartialEq<T> for test1<'a> where T: ::rust_jni::Cast<'a, ::rust_jni::java::lang::Object<'a>> {
                fn eq(&self, other: &T) -> bool {
                    self.object.eq(other)
                }
            }

            impl<'a> Eq for test1<'a> {}

            impl<'a> e::f::test3 for test1<'a> {
            }

            impl<'a> e::f::test4 for test1<'a> {
            }
        };
        assert_tokens_equals(generate(input), expected);
    }

    #[test]
    fn one_interface() {
        let input = GeneratorData {
            definitions: vec![GeneratorDefinition::Interface(
                InterfaceGeneratorDefinition {
                    interface: Ident::new("test1", Span::call_site()),
                    public: quote!{test_public},
                    extends: vec![],
                },
            )],
        };
        let expected = quote!{
            test_public trait test1 {
            }
        };
        assert_tokens_equals(generate(input), expected);
    }

    #[test]
    fn one_interface_extends() {
        let input = GeneratorData {
            definitions: vec![GeneratorDefinition::Interface(
                InterfaceGeneratorDefinition {
                    interface: Ident::new("test1", Span::call_site()),
                    public: TokenStream::new(),
                    extends: vec![quote!{c::d::test2}, quote!{e::f::test3}],
                },
            )],
        };
        let expected = quote!{
            trait test1 : c::d::test2 + e::f::test3 {
            }
        };
        assert_tokens_equals(generate(input), expected);
    }

    #[test]
    fn multiple() {
        let input = GeneratorData {
            definitions: vec![
                GeneratorDefinition::Interface(InterfaceGeneratorDefinition {
                    interface: Ident::new("test_if1", Span::call_site()),
                    public: TokenStream::new(),
                    extends: vec![],
                }),
                GeneratorDefinition::Interface(InterfaceGeneratorDefinition {
                    interface: Ident::new("test_if2", Span::call_site()),
                    public: TokenStream::new(),
                    extends: vec![],
                }),
                GeneratorDefinition::Class(ClassGeneratorDefinition {
                    class: Ident::new("test1", Span::call_site()),
                    public: TokenStream::new(),
                    super_class: quote!{c::d::test3},
                    transitive_extends: vec![quote!{c::d::test3}],
                    implements: vec![],
                    signature: Literal::string("test/sign1"),
                    full_signature: Literal::string("test/signature1"),
                    methods: vec![],
                    static_methods: vec![],
                    constructors: vec![],
                }),
                GeneratorDefinition::Class(ClassGeneratorDefinition {
                    class: Ident::new("test2", Span::call_site()),
                    public: TokenStream::new(),
                    super_class: quote!{c::d::test4},
                    transitive_extends: vec![quote!{c::d::test4}],
                    implements: vec![],
                    signature: Literal::string("test/sign2"),
                    full_signature: Literal::string("test/signature2"),
                    methods: vec![],
                    static_methods: vec![],
                    constructors: vec![],
                }),
            ],
        };
        let expected = quote!{
            trait test_if1 {
            }

            trait test_if2 {
            }

            #[derive(Debug)]
            struct test1<'env> {
                object: c::d::test3<'env>,
            }

            impl<'a> ::rust_jni::JavaType for test1<'a> {
                #[doc(hidden)]
                type __JniType = <::rust_jni::java::lang::Object<'a> as ::rust_jni::JavaType>::__JniType;

                #[doc(hidden)]
                fn __signature() -> &'static str {
                    "test/signature1"
                }
            }

            impl<'a> ::rust_jni::__generator::ToJni for test1<'a> {
                unsafe fn __to_jni(&self) -> Self::__JniType {
                    self.raw_object()
                }
            }

            impl<'a> ::rust_jni::__generator::FromJni<'a> for test1<'a> {
                unsafe fn __from_jni(env: &'a ::rust_jni::JniEnv<'a>, value: Self::__JniType) -> Self {
                    Self {
                        object: <c::d::test3 as ::rust_jni::__generator::FromJni<'a>>::__from_jni(env, value),
                    }
                }
            }

            impl<'a> ::rust_jni::Cast<'a, test1<'a>> for test1<'a> {
                #[doc(hidden)]
                fn cast<'b>(&'b self) -> &'b test1<'a> {
                    self
                }
            }

            impl<'a> ::rust_jni::Cast<'a, c::d::test3<'a>> for test1<'a> {
                #[doc(hidden)]
                fn cast<'b>(&'b self) -> &'b c::d::test3<'a> {
                    self
                }
            }

            impl<'a> ::std::ops::Deref for test1<'a> {
                type Target = c::d::test3<'a>;

                fn deref(&self) -> &Self::Target {
                    &self.object
                }
            }

            impl<'a> test1<'a> {
                pub fn get_class(env: &'a ::rust_jni::JniEnv<'a>, token: &::rust_jni::NoException<'a>)
                    -> ::rust_jni::JavaResult<'a, ::rust_jni::java::lang::Class<'a>> {
                    ::rust_jni::java::lang::Class::find(env, "test/sign1", token)
                }

                pub fn clone(&self, token: &::rust_jni::NoException<'a>) -> ::rust_jni::JavaResult<'a, Self>
                where
                    Self: Sized,
                {
                    self.object
                        .clone(token)
                        .map(|object| Self { object })
                }

                pub fn to_string(&self, token: &::rust_jni::NoException<'a>)
                    -> ::rust_jni::JavaResult<'a, ::rust_jni::java::lang::String<'a>> {
                    self.object.to_string(token)
                }
            }

            impl<'a> ::std::fmt::Display for test1<'a> {
                fn fmt(&self, formatter: &mut ::std::fmt::Formatter) -> ::std::fmt::Result {
                    self.object.fmt(formatter)
                }
            }

            impl<'a, T> PartialEq<T> for test1<'a> where T: ::rust_jni::Cast<'a, ::rust_jni::java::lang::Object<'a>> {
                fn eq(&self, other: &T) -> bool {
                    self.object.eq(other)
                }
            }

            impl<'a> Eq for test1<'a> {}

            #[derive(Debug)]
            struct test2<'env> {
                object: c::d::test4<'env>,
            }

            impl<'a> ::rust_jni::JavaType for test2<'a> {
                #[doc(hidden)]
                type __JniType = <::rust_jni::java::lang::Object<'a> as ::rust_jni::JavaType>::__JniType;

                #[doc(hidden)]
                fn __signature() -> &'static str {
                    "test/signature2"
                }
            }

            impl<'a> ::rust_jni::__generator::ToJni for test2<'a> {
                unsafe fn __to_jni(&self) -> Self::__JniType {
                    self.raw_object()
                }
            }

            impl<'a> ::rust_jni::__generator::FromJni<'a> for test2<'a> {
                unsafe fn __from_jni(env: &'a ::rust_jni::JniEnv<'a>, value: Self::__JniType) -> Self {
                    Self {
                        object: <c::d::test4 as ::rust_jni::__generator::FromJni<'a>>::__from_jni(env, value),
                    }
                }
            }

            impl<'a> ::rust_jni::Cast<'a, test2<'a>> for test2<'a> {
                #[doc(hidden)]
                fn cast<'b>(&'b self) -> &'b test2<'a> {
                    self
                }
            }

            impl<'a> ::rust_jni::Cast<'a, c::d::test4<'a>> for test2<'a> {
                #[doc(hidden)]
                fn cast<'b>(&'b self) -> &'b c::d::test4<'a> {
                    self
                }
            }

            impl<'a> ::std::ops::Deref for test2<'a> {
                type Target = c::d::test4<'a>;

                fn deref(&self) -> &Self::Target {
                    &self.object
                }
            }

            impl<'a> test2<'a> {
                pub fn get_class(env: &'a ::rust_jni::JniEnv<'a>, token: &::rust_jni::NoException<'a>)
                    -> ::rust_jni::JavaResult<'a, ::rust_jni::java::lang::Class<'a>> {
                    ::rust_jni::java::lang::Class::find(env, "test/sign2", token)
                }

                pub fn clone(&self, token: &::rust_jni::NoException<'a>) -> ::rust_jni::JavaResult<'a, Self>
                where
                    Self: Sized,
                {
                    self.object
                        .clone(token)
                        .map(|object| Self { object })
                }

                pub fn to_string(&self, token: &::rust_jni::NoException<'a>)
                    -> ::rust_jni::JavaResult<'a, ::rust_jni::java::lang::String<'a>> {
                    self.object.to_string(token)
                }
            }

            impl<'a> ::std::fmt::Display for test2<'a> {
                fn fmt(&self, formatter: &mut ::std::fmt::Formatter) -> ::std::fmt::Result {
                    self.object.fmt(formatter)
                }
            }

            impl<'a, T> PartialEq<T> for test2<'a> where T: ::rust_jni::Cast<'a, ::rust_jni::java::lang::Object<'a>> {
                fn eq(&self, other: &T) -> bool {
                    self.object.eq(other)
                }
            }

            impl<'a> Eq for test2<'a> {}
        };
        assert_tokens_equals(generate(input), expected);
    }
}

#[cfg(test)]
mod java_generate_tests {
    use super::*;

    #[test]
    fn empty() {
        let input = quote!{};
        let expected = quote!{};
        assert_tokens_equals(java_generate_impl(input), expected);
    }

    #[test]
    fn one_class() {
        let input = quote!{
            class TestClass1 extends TestClass2 {}
        };
        let expected = quote!{
            #[derive(Debug)]
            struct TestClass1<'env> {
                object: ::TestClass2<'env>,
            }

            impl<'a> ::rust_jni::JavaType for TestClass1<'a> {
                #[doc(hidden)]
                type __JniType = <::rust_jni::java::lang::Object<'a> as ::rust_jni::JavaType>::__JniType;

                #[doc(hidden)]
                fn __signature() -> &'static str {
                    "LTestClass1;"
                }
            }

            impl<'a> ::rust_jni::__generator::ToJni for TestClass1<'a> {
                unsafe fn __to_jni(&self) -> Self::__JniType {
                    self.raw_object()
                }
            }

            impl<'a> ::rust_jni::__generator::FromJni<'a> for TestClass1<'a> {
                unsafe fn __from_jni(env: &'a ::rust_jni::JniEnv<'a>, value: Self::__JniType) -> Self {
                    Self {
                        object: <::TestClass2 as ::rust_jni::__generator::FromJni<'a>>::__from_jni(env, value),
                    }
                }
            }

            impl<'a> ::rust_jni::Cast<'a, TestClass1<'a>> for TestClass1<'a> {
                #[doc(hidden)]
                fn cast<'b>(&'b self) -> &'b TestClass1<'a> {
                    self
                }
            }

            impl<'a> ::rust_jni::Cast<'a, ::TestClass2<'a>> for TestClass1<'a> {
                #[doc(hidden)]
                fn cast<'b>(&'b self) -> &'b ::TestClass2<'a> {
                    self
                }
            }

            impl<'a> ::std::ops::Deref for TestClass1<'a> {
                type Target = ::TestClass2<'a>;

                fn deref(&self) -> &Self::Target {
                    &self.object
                }
            }

            impl<'a> TestClass1<'a> {
                pub fn get_class(env: &'a ::rust_jni::JniEnv<'a>, token: &::rust_jni::NoException<'a>)
                    -> ::rust_jni::JavaResult<'a, ::rust_jni::java::lang::Class<'a>> {
                    ::rust_jni::java::lang::Class::find(env, "TestClass1", token)
                }

                pub fn clone(&self, token: &::rust_jni::NoException<'a>) -> ::rust_jni::JavaResult<'a, Self>
                where
                    Self: Sized,
                {
                    self.object
                        .clone(token)
                        .map(|object| Self { object })
                }

                pub fn to_string(&self, token: &::rust_jni::NoException<'a>)
                    -> ::rust_jni::JavaResult<'a, ::rust_jni::java::lang::String<'a>> {
                    self.object.to_string(token)
                }
            }

            impl<'a> ::std::fmt::Display for TestClass1<'a> {
                fn fmt(&self, formatter: &mut ::std::fmt::Formatter) -> ::std::fmt::Result {
                    self.object.fmt(formatter)
                }
            }

            impl<'a, T> PartialEq<T> for TestClass1<'a> where T: ::rust_jni::Cast<'a, ::rust_jni::java::lang::Object<'a>> {
                fn eq(&self, other: &T) -> bool {
                    self.object.eq(other)
                }
            }

            impl<'a> Eq for TestClass1<'a> {}
        };
        assert_tokens_equals(java_generate_impl(input), expected);
    }

    #[test]
    fn one_class_implements() {
        let input = quote!{
            interface a.b.TestInterface1 {}
            interface a.b.TestInterface2 {}
            class TestClass1 extends TestClass2 implements a.b.TestInterface1, a.b.TestInterface2 {}
        };
        let expected = quote!{
            trait TestInterface1 {
            }

            trait TestInterface2 {
            }

            #[derive(Debug)]
            struct TestClass1<'env> {
                object: ::TestClass2<'env>,
            }

            impl<'a> ::rust_jni::JavaType for TestClass1<'a> {
                #[doc(hidden)]
                type __JniType = <::rust_jni::java::lang::Object<'a> as ::rust_jni::JavaType>::__JniType;

                #[doc(hidden)]
                fn __signature() -> &'static str {
                    "LTestClass1;"
                }
            }

            impl<'a> ::rust_jni::__generator::ToJni for TestClass1<'a> {
                unsafe fn __to_jni(&self) -> Self::__JniType {
                    self.raw_object()
                }
            }

            impl<'a> ::rust_jni::__generator::FromJni<'a> for TestClass1<'a> {
                unsafe fn __from_jni(env: &'a ::rust_jni::JniEnv<'a>, value: Self::__JniType) -> Self {
                    Self {
                        object: <::TestClass2 as ::rust_jni::__generator::FromJni<'a>>::__from_jni(env, value),
                    }
                }
            }

            impl<'a> ::rust_jni::Cast<'a, TestClass1<'a>> for TestClass1<'a> {
                #[doc(hidden)]
                fn cast<'b>(&'b self) -> &'b TestClass1<'a> {
                    self
                }
            }

            impl<'a> ::rust_jni::Cast<'a, ::TestClass2<'a>> for TestClass1<'a> {
                #[doc(hidden)]
                fn cast<'b>(&'b self) -> &'b ::TestClass2<'a> {
                    self
                }
            }

            impl<'a> ::std::ops::Deref for TestClass1<'a> {
                type Target = ::TestClass2<'a>;

                fn deref(&self) -> &Self::Target {
                    &self.object
                }
            }

            impl<'a> TestClass1<'a> {
                pub fn get_class(env: &'a ::rust_jni::JniEnv<'a>, token: &::rust_jni::NoException<'a>)
                    -> ::rust_jni::JavaResult<'a, ::rust_jni::java::lang::Class<'a>> {
                    ::rust_jni::java::lang::Class::find(env, "TestClass1", token)
                }

                pub fn clone(&self, token: &::rust_jni::NoException<'a>) -> ::rust_jni::JavaResult<'a, Self>
                where
                    Self: Sized,
                {
                    self.object
                        .clone(token)
                        .map(|object| Self { object })
                }

                pub fn to_string(&self, token: &::rust_jni::NoException<'a>)
                    -> ::rust_jni::JavaResult<'a, ::rust_jni::java::lang::String<'a>> {
                    self.object.to_string(token)
                }
            }

            impl<'a> ::std::fmt::Display for TestClass1<'a> {
                fn fmt(&self, formatter: &mut ::std::fmt::Formatter) -> ::std::fmt::Result {
                    self.object.fmt(formatter)
                }
            }

            impl<'a, T> PartialEq<T> for TestClass1<'a> where T: ::rust_jni::Cast<'a, ::rust_jni::java::lang::Object<'a>> {
                fn eq(&self, other: &T) -> bool {
                    self.object.eq(other)
                }
            }

            impl<'a> Eq for TestClass1<'a> {}

            impl<'a> ::a::b::TestInterface1 for TestClass1<'a> {
            }

            impl<'a> ::a::b::TestInterface2 for TestClass1<'a> {
            }
        };
        assert_tokens_equals(java_generate_impl(input), expected);
    }

    #[test]
    fn one_class_packaged() {
        let input = quote!{
            class a.b.TestClass1 extends c.d.TestClass2 {}
        };
        let expected = quote!{
            #[derive(Debug)]
            struct TestClass1<'env> {
                object: ::c::d::TestClass2<'env>,
            }

            impl<'a> ::rust_jni::JavaType for TestClass1<'a> {
                #[doc(hidden)]
                type __JniType = <::rust_jni::java::lang::Object<'a> as ::rust_jni::JavaType>::__JniType;

                #[doc(hidden)]
                fn __signature() -> &'static str {
                    "La/b/TestClass1;"
                }
            }

            impl<'a> ::rust_jni::__generator::ToJni for TestClass1<'a> {
                unsafe fn __to_jni(&self) -> Self::__JniType {
                    self.raw_object()
                }
            }

            impl<'a> ::rust_jni::__generator::FromJni<'a> for TestClass1<'a> {
                unsafe fn __from_jni(env: &'a ::rust_jni::JniEnv<'a>, value: Self::__JniType) -> Self {
                    Self {
                        object: <::c::d::TestClass2 as ::rust_jni::__generator::FromJni<'a>>::__from_jni(env, value),
                    }
                }
            }

            impl<'a> ::rust_jni::Cast<'a, TestClass1<'a>> for TestClass1<'a> {
                #[doc(hidden)]
                fn cast<'b>(&'b self) -> &'b TestClass1<'a> {
                    self
                }
            }

            impl<'a> ::rust_jni::Cast<'a, ::c::d::TestClass2<'a>> for TestClass1<'a> {
                #[doc(hidden)]
                fn cast<'b>(&'b self) -> &'b ::c::d::TestClass2<'a> {
                    self
                }
            }

            impl<'a> ::std::ops::Deref for TestClass1<'a> {
                type Target = ::c::d::TestClass2<'a>;

                fn deref(&self) -> &Self::Target {
                    &self.object
                }
            }

            impl<'a> TestClass1<'a> {
                pub fn get_class(env: &'a ::rust_jni::JniEnv<'a>, token: &::rust_jni::NoException<'a>)
                    -> ::rust_jni::JavaResult<'a, ::rust_jni::java::lang::Class<'a>> {
                    ::rust_jni::java::lang::Class::find(env, "a/b/TestClass1", token)
                }

                pub fn clone(&self, token: &::rust_jni::NoException<'a>) -> ::rust_jni::JavaResult<'a, Self>
                where
                    Self: Sized,
                {
                    self.object
                        .clone(token)
                        .map(|object| Self { object })
                }

                pub fn to_string(&self, token: &::rust_jni::NoException<'a>)
                    -> ::rust_jni::JavaResult<'a, ::rust_jni::java::lang::String<'a>> {
                    self.object.to_string(token)
                }
            }

            impl<'a> ::std::fmt::Display for TestClass1<'a> {
                fn fmt(&self, formatter: &mut ::std::fmt::Formatter) -> ::std::fmt::Result {
                    self.object.fmt(formatter)
                }
            }

            impl<'a, T> PartialEq<T> for TestClass1<'a> where T: ::rust_jni::Cast<'a, ::rust_jni::java::lang::Object<'a>> {
                fn eq(&self, other: &T) -> bool {
                    self.object.eq(other)
                }
            }

            impl<'a> Eq for TestClass1<'a> {}
        };
        assert_tokens_equals(java_generate_impl(input), expected);
    }

    #[test]
    fn one_class_public() {
        let input = quote!{
            public class TestClass1 extends TestClass2 {}
        };
        let expected = quote!{
            #[derive(Debug)]
            pub struct TestClass1<'env> {
                object: ::TestClass2<'env>,
            }

            impl<'a> ::rust_jni::JavaType for TestClass1<'a> {
                #[doc(hidden)]
                type __JniType = <::rust_jni::java::lang::Object<'a> as ::rust_jni::JavaType>::__JniType;

                #[doc(hidden)]
                fn __signature() -> &'static str {
                    "LTestClass1;"
                }
            }

            impl<'a> ::rust_jni::__generator::ToJni for TestClass1<'a> {
                unsafe fn __to_jni(&self) -> Self::__JniType {
                    self.raw_object()
                }
            }

            impl<'a> ::rust_jni::__generator::FromJni<'a> for TestClass1<'a> {
                unsafe fn __from_jni(env: &'a ::rust_jni::JniEnv<'a>, value: Self::__JniType) -> Self {
                    Self {
                        object: <::TestClass2 as ::rust_jni::__generator::FromJni<'a>>::__from_jni(env, value),
                    }
                }
            }

            impl<'a> ::rust_jni::Cast<'a, TestClass1<'a>> for TestClass1<'a> {
                #[doc(hidden)]
                fn cast<'b>(&'b self) -> &'b TestClass1<'a> {
                    self
                }
            }

            impl<'a> ::rust_jni::Cast<'a, ::TestClass2<'a>> for TestClass1<'a> {
                #[doc(hidden)]
                fn cast<'b>(&'b self) -> &'b ::TestClass2<'a> {
                    self
                }
            }

            impl<'a> ::std::ops::Deref for TestClass1<'a> {
                type Target = ::TestClass2<'a>;

                fn deref(&self) -> &Self::Target {
                    &self.object
                }
            }

            impl<'a> TestClass1<'a> {
                pub fn get_class(env: &'a ::rust_jni::JniEnv<'a>, token: &::rust_jni::NoException<'a>)
                    -> ::rust_jni::JavaResult<'a, ::rust_jni::java::lang::Class<'a>> {
                    ::rust_jni::java::lang::Class::find(env, "TestClass1", token)
                }

                pub fn clone(&self, token: &::rust_jni::NoException<'a>) -> ::rust_jni::JavaResult<'a, Self>
                where
                    Self: Sized,
                {
                    self.object
                        .clone(token)
                        .map(|object| Self { object })
                }

                pub fn to_string(&self, token: &::rust_jni::NoException<'a>)
                    -> ::rust_jni::JavaResult<'a, ::rust_jni::java::lang::String<'a>> {
                    self.object.to_string(token)
                }
            }

            impl<'a> ::std::fmt::Display for TestClass1<'a> {
                fn fmt(&self, formatter: &mut ::std::fmt::Formatter) -> ::std::fmt::Result {
                    self.object.fmt(formatter)
                }
            }

            impl<'a, T> PartialEq<T> for TestClass1<'a> where T: ::rust_jni::Cast<'a, ::rust_jni::java::lang::Object<'a>> {
                fn eq(&self, other: &T) -> bool {
                    self.object.eq(other)
                }
            }

            impl<'a> Eq for TestClass1<'a> {}
        };
        assert_tokens_equals(java_generate_impl(input), expected);
    }

    #[test]
    fn one_interface() {
        let input = quote!{
            interface TestInterface1 {}
        };
        let expected = quote!{
            trait TestInterface1 {
            }
        };
        assert_tokens_equals(java_generate_impl(input), expected);
    }

    #[test]
    fn one_interface_packaged() {
        let input = quote!{
            interface a.b.TestInterface1 {}
        };
        let expected = quote!{
            trait TestInterface1 {
            }
        };
        assert_tokens_equals(java_generate_impl(input), expected);
    }

    #[test]
    fn one_interface_public() {
        let input = quote!{
            public interface TestInterface1 {}
        };
        let expected = quote!{
            pub trait TestInterface1 {
            }
        };
        assert_tokens_equals(java_generate_impl(input), expected);
    }

    #[test]
    fn one_interface_extends() {
        let input = quote!{
            interface TestInterface2 {}
            interface TestInterface3 {}
            interface TestInterface1 extends TestInterface2, TestInterface3 {}
        };
        let expected = quote!{
            trait TestInterface2 {
            }

            trait TestInterface3 {
            }

            trait TestInterface1: ::TestInterface2 + ::TestInterface3 {
            }
        };
        assert_tokens_equals(java_generate_impl(input), expected);
    }

    #[test]
    fn multiple() {
        let input = quote!{
            interface TestInterface1 {}
            interface TestInterface2 {}
            class TestClass1 {}
            class TestClass2 {}

            metadata {
                interface TestInterface3 {}
                class TestClass3;
            }
        };
        let expected = quote!{
            trait TestInterface1 {
            }

            trait TestInterface2 {
            }

            #[derive(Debug)]
            struct TestClass1<'env> {
                object: ::java::lang::Object<'env>,
            }

            impl<'a> ::rust_jni::JavaType for TestClass1<'a> {
                #[doc(hidden)]
                type __JniType = <::rust_jni::java::lang::Object<'a> as ::rust_jni::JavaType>::__JniType;

                #[doc(hidden)]
                fn __signature() -> &'static str {
                    "LTestClass1;"
                }
            }

            impl<'a> ::rust_jni::__generator::ToJni for TestClass1<'a> {
                unsafe fn __to_jni(&self) -> Self::__JniType {
                    self.raw_object()
                }
            }

            impl<'a> ::rust_jni::__generator::FromJni<'a> for TestClass1<'a> {
                unsafe fn __from_jni(env: &'a ::rust_jni::JniEnv<'a>, value: Self::__JniType) -> Self {
                    Self {
                        object: <::java::lang::Object as ::rust_jni::__generator::FromJni<'a>>::__from_jni(env, value),
                    }
                }
            }

            impl<'a> ::rust_jni::Cast<'a, TestClass1<'a>> for TestClass1<'a> {
                #[doc(hidden)]
                fn cast<'b>(&'b self) -> &'b TestClass1<'a> {
                    self
                }
            }

            impl<'a> ::rust_jni::Cast<'a, ::java::lang::Object<'a>> for TestClass1<'a> {
                #[doc(hidden)]
                fn cast<'b>(&'b self) -> &'b ::java::lang::Object<'a> {
                    self
                }
            }

            impl<'a> ::std::ops::Deref for TestClass1<'a> {
                type Target = ::java::lang::Object<'a>;

                fn deref(&self) -> &Self::Target {
                    &self.object
                }
            }

            impl<'a> TestClass1<'a> {
                pub fn get_class(env: &'a ::rust_jni::JniEnv<'a>, token: &::rust_jni::NoException<'a>)
                    -> ::rust_jni::JavaResult<'a, ::rust_jni::java::lang::Class<'a>> {
                    ::rust_jni::java::lang::Class::find(env, "TestClass1", token)
                }

                pub fn clone(&self, token: &::rust_jni::NoException<'a>) -> ::rust_jni::JavaResult<'a, Self>
                where
                    Self: Sized,
                {
                    self.object
                        .clone(token)
                        .map(|object| Self { object })
                }

                pub fn to_string(&self, token: &::rust_jni::NoException<'a>)
                    -> ::rust_jni::JavaResult<'a, ::rust_jni::java::lang::String<'a>> {
                    self.object.to_string(token)
                }
            }

            impl<'a> ::std::fmt::Display for TestClass1<'a> {
                fn fmt(&self, formatter: &mut ::std::fmt::Formatter) -> ::std::fmt::Result {
                    self.object.fmt(formatter)
                }
            }

            impl<'a, T> PartialEq<T> for TestClass1<'a> where T: ::rust_jni::Cast<'a, ::rust_jni::java::lang::Object<'a>> {
                fn eq(&self, other: &T) -> bool {
                    self.object.eq(other)
                }
            }

            impl<'a> Eq for TestClass1<'a> {}

            #[derive(Debug)]
            struct TestClass2<'env> {
                object: ::java::lang::Object<'env>,
            }

            impl<'a> ::rust_jni::JavaType for TestClass2<'a> {
                #[doc(hidden)]
                type __JniType = <::rust_jni::java::lang::Object<'a> as ::rust_jni::JavaType>::__JniType;

                #[doc(hidden)]
                fn __signature() -> &'static str {
                    "LTestClass2;"
                }
            }

            impl<'a> ::rust_jni::__generator::ToJni for TestClass2<'a> {
                unsafe fn __to_jni(&self) -> Self::__JniType {
                    self.raw_object()
                }
            }

            impl<'a> ::rust_jni::__generator::FromJni<'a> for TestClass2<'a> {
                unsafe fn __from_jni(env: &'a ::rust_jni::JniEnv<'a>, value: Self::__JniType) -> Self {
                    Self {
                        object: <::java::lang::Object as ::rust_jni::__generator::FromJni<'a>>::__from_jni(env, value),
                    }
                }
            }

            impl<'a> ::rust_jni::Cast<'a, TestClass2<'a>> for TestClass2<'a> {
                #[doc(hidden)]
                fn cast<'b>(&'b self) -> &'b TestClass2<'a> {
                    self
                }
            }

            impl<'a> ::rust_jni::Cast<'a, ::java::lang::Object<'a>> for TestClass2<'a> {
                #[doc(hidden)]
                fn cast<'b>(&'b self) -> &'b ::java::lang::Object<'a> {
                    self
                }
            }

            impl<'a> ::std::ops::Deref for TestClass2<'a> {
                type Target = ::java::lang::Object<'a>;

                fn deref(&self) -> &Self::Target {
                    &self.object
                }
            }

            impl<'a> TestClass2<'a> {
                pub fn get_class(env: &'a ::rust_jni::JniEnv<'a>, token: &::rust_jni::NoException<'a>)
                    -> ::rust_jni::JavaResult<'a, ::rust_jni::java::lang::Class<'a>> {
                    ::rust_jni::java::lang::Class::find(env, "TestClass2", token)
                }

                pub fn clone(&self, token: &::rust_jni::NoException<'a>) -> ::rust_jni::JavaResult<'a, Self>
                where
                    Self: Sized,
                {
                    self.object
                        .clone(token)
                        .map(|object| Self { object })
                }

                pub fn to_string(&self, token: &::rust_jni::NoException<'a>)
                    -> ::rust_jni::JavaResult<'a, ::rust_jni::java::lang::String<'a>> {
                    self.object.to_string(token)
                }
            }

            impl<'a> ::std::fmt::Display for TestClass2<'a> {
                fn fmt(&self, formatter: &mut ::std::fmt::Formatter) -> ::std::fmt::Result {
                    self.object.fmt(formatter)
                }
            }

            impl<'a, T> PartialEq<T> for TestClass2<'a> where T: ::rust_jni::Cast<'a, ::rust_jni::java::lang::Object<'a>> {
                fn eq(&self, other: &T) -> bool {
                    self.object.eq(other)
                }
            }

            impl<'a> Eq for TestClass2<'a> {}
        };
        assert_tokens_equals(java_generate_impl(input), expected);
    }

    #[test]
    fn integration() {
        let input = quote!{
            public interface a.b.TestInterface3 {}
            public interface a.b.TestInterface4 extends c.d.TestInterface2, a.b.TestInterface3 {}

            public class a.b.TestClass3 extends c.d.TestClass2 implements e.f.TestInterface1, a.b.TestInterface4 {
                public a.b.TestClass3(int arg1, a.b.TestClass3 arg2);

                long primitiveFunc3(int arg1, char arg2);
                public c.d.TestClass2 objectFunc3(a.b.TestClass3 arg);

                static long primitiveStaticFunc3(int arg1, char arg2);
                public static c.d.TestClass2 objectStaticFunc3(a.b.TestClass3 arg);
            }

            metadata {
                interface e.f.TestInterface1 {}
                interface c.d.TestInterface2 extends e.f.TestInterface1 {}

                class c.d.TestClass1;
                class c.d.TestClass2 extends c.d.TestClass1 implements e.f.TestInterface1;
            }
        };
        let expected = quote!{
            pub trait TestInterface3 {
            }

            pub trait TestInterface4: ::c::d::TestInterface2 + ::a::b::TestInterface3 {
            }

            #[derive(Debug)]
            pub struct TestClass3<'env> {
                object: ::c::d::TestClass2<'env>,
            }

            impl<'a> ::rust_jni::JavaType for TestClass3<'a> {
                #[doc(hidden)]
                type __JniType = <::rust_jni::java::lang::Object<'a> as ::rust_jni::JavaType>::__JniType;

                #[doc(hidden)]
                fn __signature() -> &'static str {
                    "La/b/TestClass3;"
                }
            }

            impl<'a> ::rust_jni::__generator::ToJni for TestClass3<'a> {
                unsafe fn __to_jni(&self) -> Self::__JniType {
                    self.raw_object()
                }
            }

            impl<'a> ::rust_jni::__generator::FromJni<'a> for TestClass3<'a> {
                unsafe fn __from_jni(env: &'a ::rust_jni::JniEnv<'a>, value: Self::__JniType) -> Self {
                    Self {
                        object: <::c::d::TestClass2 as ::rust_jni::__generator::FromJni<'a>>::__from_jni(env, value),
                    }
                }
            }

            impl<'a> ::rust_jni::Cast<'a, TestClass3<'a>> for TestClass3<'a> {
                #[doc(hidden)]
                fn cast<'b>(&'b self) -> &'b TestClass3<'a> {
                    self
                }
            }

            impl<'a> ::rust_jni::Cast<'a, ::c::d::TestClass2<'a>> for TestClass3<'a> {
                #[doc(hidden)]
                fn cast<'b>(&'b self) -> &'b ::c::d::TestClass2<'a> {
                    self
                }
            }

            impl<'a> ::rust_jni::Cast<'a, ::c::d::TestClass1<'a>> for TestClass3<'a> {
                #[doc(hidden)]
                fn cast<'b>(&'b self) -> &'b ::c::d::TestClass1<'a> {
                    self
                }
            }

            impl<'a> ::rust_jni::Cast<'a, ::java::lang::Object<'a>> for TestClass3<'a> {
                #[doc(hidden)]
                fn cast<'b>(&'b self) -> &'b ::java::lang::Object<'a> {
                    self
                }
            }

            impl<'a> ::std::ops::Deref for TestClass3<'a> {
                type Target = ::c::d::TestClass2<'a>;

                fn deref(&self) -> &Self::Target {
                    &self.object
                }
            }

            impl<'a> TestClass3<'a> {
                pub fn get_class(env: &'a ::rust_jni::JniEnv<'a>, token: &::rust_jni::NoException<'a>)
                    -> ::rust_jni::JavaResult<'a, ::rust_jni::java::lang::Class<'a>> {
                    ::rust_jni::java::lang::Class::find(env, "a/b/TestClass3", token)
                }

                pub fn clone(&self, token: &::rust_jni::NoException<'a>) -> ::rust_jni::JavaResult<'a, Self>
                where
                    Self: Sized,
                {
                    self.object
                        .clone(token)
                        .map(|object| Self { object })
                }

                pub fn to_string(&self, token: &::rust_jni::NoException<'a>)
                    -> ::rust_jni::JavaResult<'a, ::rust_jni::java::lang::String<'a>> {
                    self.object.to_string(token)
                }

                pub fn new(
                    env: &'a ::rust_jni::JniEnv<'a>,
                    arg1: i32,
                    arg2: &::a::b::TestClass3,
                    token: &::rust_jni::NoException<'a>,
                ) -> ::rust_jni::JavaResult<'a, Self> {
                    // Safe because the method name and arguments are correct.
                    unsafe {
                        ::rust_jni::__generator::call_constructor::<Self, _, fn(i32, &::a::b::TestClass3,)>
                        (
                            env,
                            (arg1, arg2,),
                            token,
                        )
                    }
                }

                fn primitiveFunc3(
                    &self,
                    arg1: i32,
                    arg2: char,
                    token: &::rust_jni::NoException<'a>,
                ) -> ::rust_jni::JavaResult<'a, i64> {
                    // Safe because the method name and arguments are correct.
                    unsafe {
                        ::rust_jni::__generator::call_method::<_, _, _,
                            fn(i32, char,) -> i64
                        >
                        (
                            self,
                            "primitiveFunc3",
                            (arg1, arg2,),
                            token,
                        )
                    }
                }

                pub fn objectFunc3(
                    &self,
                    arg: &::a::b::TestClass3,
                    token: &::rust_jni::NoException<'a>,
                ) -> ::rust_jni::JavaResult<'a, ::c::d::TestClass2<'a> > {
                    // Safe because the method name and arguments are correct.
                    unsafe {
                        ::rust_jni::__generator::call_method::<_, _, _,
                            fn(&::a::b::TestClass3,) -> ::c::d::TestClass2<'a>
                        >
                        (
                            self,
                            "objectFunc3",
                            (arg,),
                            token,
                        )
                    }
                }

                fn primitiveStaticFunc3(
                    env: &'a ::rust_jni::JniEnv<'a>,
                    arg1: i32,
                    arg2: char,
                    token: &::rust_jni::NoException<'a>,
                ) -> ::rust_jni::JavaResult<'a, i64> {
                    // Safe because the method name and arguments are correct.
                    unsafe {
                        ::rust_jni::__generator::call_static_method::<Self, _, _,
                            fn(i32, char,) -> i64
                        >
                        (
                            env,
                            "primitiveStaticFunc3",
                            (arg1, arg2,),
                            token,
                        )
                    }
                }

                pub fn objectStaticFunc3(
                    env: &'a ::rust_jni::JniEnv<'a>,
                    arg: &::a::b::TestClass3,
                    token: &::rust_jni::NoException<'a>,
                ) -> ::rust_jni::JavaResult<'a, ::c::d::TestClass2<'a> > {
                    // Safe because the method name and arguments are correct.
                    unsafe {
                        ::rust_jni::__generator::call_static_method::<Self, _, _,
                            fn(&::a::b::TestClass3,) -> ::c::d::TestClass2<'a>
                        >
                        (
                            env,
                            "objectStaticFunc3",
                            (arg,),
                            token,
                        )
                    }
                }
            }

            impl<'a> ::std::fmt::Display for TestClass3<'a> {
                fn fmt(&self, formatter: &mut ::std::fmt::Formatter) -> ::std::fmt::Result {
                    self.object.fmt(formatter)
                }
            }

            impl<'a, T> PartialEq<T> for TestClass3<'a> where T: ::rust_jni::Cast<'a, ::rust_jni::java::lang::Object<'a>> {
                fn eq(&self, other: &T) -> bool {
                    self.object.eq(other)
                }
            }

            impl<'a> Eq for TestClass3<'a> {}


            impl<'a> ::a::b::TestInterface3 for TestClass3<'a> {
            }

            impl<'a> ::a::b::TestInterface4 for TestClass3<'a> {
            }

            impl<'a> ::c::d::TestInterface2 for TestClass3<'a> {
            }

            impl<'a> ::e::f::TestInterface1 for TestClass3<'a> {
            }
        };
        assert_tokens_equals(java_generate_impl(input), expected);
    }
}

#[cfg(test)]
fn assert_tokens_equals(left: TokenStream, right: TokenStream) {
    assert_eq!(format!("{:?}", left), format!("{:?}", right),);
}
