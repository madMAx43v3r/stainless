use std::str::FromStr;

use proc_macro2::{Span as TokenSpan, TokenStream};
use quote::quote;

use crate::ast::{BinaryOperator, LiteralKind, PrefixOperator};
use crate::hir;

pub(crate) fn emit(program: &hir::Program) -> Result<String, String> {
    let mut emitter = Emitter { temporary_index: 0 };
    let tokens = emitter.program(program)?;
    let file = syn::parse2::<syn::File>(tokens)
        .map_err(|error| format!("generated Rust was not a valid syntax tree: {error}"))?;
    Ok(prettyplease::unparse(&file))
}

struct Emitter {
    temporary_index: usize,
}

impl Emitter {
    fn program(&mut self, program: &hir::Program) -> Result<TokenStream, String> {
        let functions = program
            .functions
            .iter()
            .map(|function| self.function(function))
            .collect::<Result<Vec<_>, _>>()?;
        let modules = program
            .modules
            .iter()
            .map(|module| self.module(module))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(quote! {
            #(#functions)*
            #(#modules)*
        })
    }

    fn module(&mut self, module: &hir::Module) -> Result<TokenStream, String> {
        let name = identifier(&module.rust_name)?;
        let functions = module
            .functions
            .iter()
            .map(|function| self.function(function))
            .collect::<Result<Vec<_>, _>>()?;
        let modules = module
            .modules
            .iter()
            .map(|module| self.module(module))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(quote! {
            #[allow(non_snake_case)]
            pub mod #name {
                #(#functions)*
                #(#modules)*
            }
        })
    }

    fn function(&mut self, function: &hir::Function) -> Result<TokenStream, String> {
        let name = identifier(&function.rust_name)?;
        let reference_parameter_count = function
            .parameters
            .iter()
            .filter(|parameter| matches!(parameter.ty, hir::Type::Reference { .. }))
            .count();
        let return_is_reference = matches!(function.return_type, hir::Type::Reference { .. });
        if return_is_reference && reference_parameter_count == 0 {
            return Err(format!(
                "reference-returning function `{}` has no input borrow",
                function.source_path.join("::")
            ));
        }
        let explicit_lifetime = return_is_reference && reference_parameter_count > 1;
        let lifetime = syn::Lifetime::new("'__stainless_borrow", TokenSpan::call_site());
        let parameters = function
            .parameters
            .iter()
            .map(|parameter| parameter_tokens(parameter, explicit_lifetime.then_some(&lifetime)))
            .collect::<Result<Vec<_>, _>>()?;
        let return_type = type_tokens(
            &function.return_type,
            explicit_lifetime.then_some(&lifetime),
        )?;
        let body = self.block(&function.body)?;
        let generics = explicit_lifetime.then(|| quote!(<'__stainless_borrow>));
        Ok(quote! {
            #[allow(non_snake_case, unused_mut, unused_parens)]
            pub fn #name #generics (#(#parameters),*) -> #return_type #body
        })
    }

    fn block(&mut self, block: &hir::Block) -> Result<TokenStream, String> {
        let statements = block
            .statements
            .iter()
            .map(|statement| self.statement(statement))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(quote!({ #(#statements)* }))
    }

    fn statement(&mut self, statement: &hir::Statement) -> Result<TokenStream, String> {
        match statement {
            hir::Statement::Block(block) => self.block(block),
            hir::Statement::Let {
                name,
                ty,
                mutable,
                initializer,
            } => {
                let name = identifier(name)?;
                let ty = type_tokens(ty, None)?;
                let mutable = mutable.then(|| quote!(mut));
                let initializer = self.expression(initializer)?;
                Ok(quote!(let #mutable #name: #ty = #initializer;))
            }
            hir::Statement::Return(value) => {
                let value = value
                    .as_ref()
                    .map(|value| self.expression(value))
                    .transpose()?;
                Ok(quote!(return #value;))
            }
            hir::Statement::If {
                condition,
                then_branch,
                else_branch,
            } => {
                let condition = self.expression(condition)?;
                let then_branch = self.block(then_branch)?;
                let else_branch = match else_branch.as_deref() {
                    Some(statement @ hir::Statement::If { .. }) => {
                        let statement = self.statement(statement)?;
                        Some(quote!(else #statement))
                    }
                    Some(hir::Statement::Block(block)) => {
                        let block = self.block(block)?;
                        Some(quote!(else #block))
                    }
                    Some(statement) => {
                        let statement = self.statement(statement)?;
                        Some(quote!(else { #statement }))
                    }
                    None => None,
                };
                Ok(quote!(if #condition #then_branch #else_branch))
            }
            hir::Statement::ClassicFor {
                initializer,
                condition,
                update,
                body,
            } => self.classic_for(
                initializer.as_ref(),
                condition.as_ref(),
                update.as_ref(),
                body,
            ),
            hir::Statement::RangeFor {
                name,
                mutable,
                mode,
                iterable,
                body,
            } => {
                let name = identifier(name)?;
                let mutable = mutable.then(|| quote!(mut));
                let iterable = self.expression(iterable)?;
                let iterator = match mode {
                    hir::RangeMode::Shared => quote!((#iterable).iter()),
                    hir::RangeMode::Mutable => quote!((#iterable).iter_mut()),
                    hir::RangeMode::Copy => quote!((#iterable).iter().copied()),
                    hir::RangeMode::Move => quote!((#iterable).into_iter()),
                };
                let body = self.block(body)?;
                Ok(quote!(for #mutable #name in #iterator #body))
            }
            hir::Statement::Break => Ok(quote!(break;)),
            hir::Statement::Continue => Ok(quote!(continue;)),
            hir::Statement::Expression(expression) => {
                let expression = self.expression(expression)?;
                Ok(quote!(#expression;))
            }
        }
    }

    fn classic_for(
        &mut self,
        initializer: Option<&hir::ForInitializer>,
        condition: Option<&hir::Expression>,
        update: Option<&hir::Expression>,
        body: &hir::Block,
    ) -> Result<TokenStream, String> {
        let initializer = initializer
            .map(|initializer| self.for_initializer(initializer))
            .transpose()?;
        let condition = condition
            .map(|condition| self.expression(condition))
            .transpose()?;
        let condition_check = condition.map(|condition| {
            quote! {
                if !(#condition) {
                    break;
                }
            }
        });
        let body = self.block(body)?;
        if let Some(update) = update {
            let update = self.expression(update)?;
            let first = self.temporary("for_first")?;
            Ok(quote!({
                #initializer
                let mut #first = true;
                loop {
                    if #first {
                        #first = false;
                    } else {
                        #update;
                    }
                    #condition_check
                    #body
                }
            }))
        } else {
            Ok(quote!({
                #initializer
                loop {
                    #condition_check
                    #body
                }
            }))
        }
    }

    fn for_initializer(
        &mut self,
        initializer: &hir::ForInitializer,
    ) -> Result<TokenStream, String> {
        match initializer {
            hir::ForInitializer::Let {
                name,
                ty,
                mutable,
                initializer,
            } => {
                let name = identifier(name)?;
                let ty = type_tokens(ty, None)?;
                let mutable = mutable.then(|| quote!(mut));
                let initializer = self.expression(initializer)?;
                Ok(quote!(let #mutable #name: #ty = #initializer;))
            }
            hir::ForInitializer::Expression(expression) => {
                let expression = self.expression(expression)?;
                Ok(quote!(#expression;))
            }
        }
    }

    fn expression(&mut self, expression: &hir::Expression) -> Result<TokenStream, String> {
        match expression {
            hir::Expression::Name(name) => {
                let name = identifier(name)?;
                Ok(quote!(#name))
            }
            hir::Expression::Literal { kind, text } => literal(*kind, text),
            hir::Expression::Parenthesized(expression) => {
                let expression = self.expression(expression)?;
                Ok(quote!((#expression)))
            }
            hir::Expression::Borrow {
                mutable,
                expression,
            } => {
                let mutable = mutable.then(|| quote!(mut));
                let expression = self.expression(expression)?;
                Ok(quote!(& #mutable #expression))
            }
            hir::Expression::Dereference(expression) => {
                let expression = self.expression(expression)?;
                Ok(quote!(*(#expression)))
            }
            hir::Expression::Prefix { operator, operand } => {
                let operand = self.expression(operand)?;
                match operator {
                    PrefixOperator::Plus => Ok(quote!((#operand))),
                    PrefixOperator::Negate => Ok(quote!(-(#operand))),
                    PrefixOperator::Not | PrefixOperator::BitwiseNot => Ok(quote!(!(#operand))),
                    PrefixOperator::Increment | PrefixOperator::Decrement => {
                        Err("increment reached the direct prefix emitter".to_owned())
                    }
                }
            }
            hir::Expression::Increment {
                place,
                increment,
                prefix,
            } => {
                let place = self.expression(place)?;
                let operator = if *increment { quote!(+=) } else { quote!(-=) };
                if *prefix {
                    Ok(quote!({
                        #place #operator 1;
                        #place
                    }))
                } else {
                    let previous = self.temporary("previous")?;
                    Ok(quote!({
                        let #previous = #place;
                        #place #operator 1;
                        #previous
                    }))
                }
            }
            hir::Expression::Binary {
                left,
                operator,
                right,
            } => {
                let left = self.expression(left)?;
                let right = self.expression(right)?;
                let operator = binary_operator(*operator);
                Ok(quote!((#left #operator #right)))
            }
            hir::Expression::FunctionCall { .. }
            | hir::Expression::AssociatedCall { .. }
            | hir::Expression::MethodCall { .. }
            | hir::Expression::Clone { .. }
            | hir::Expression::Cast { .. } => self.call_expression(expression),
        }
    }

    fn call_expression(&mut self, expression: &hir::Expression) -> Result<TokenStream, String> {
        match expression {
            hir::Expression::FunctionCall {
                modules,
                function,
                arguments,
            } => {
                let target = if modules.is_empty() {
                    format!("crate::{function}")
                } else {
                    format!("crate::{}::{function}", modules.join("::"))
                };
                let target = path(&target)?;
                let arguments = arguments
                    .iter()
                    .map(|argument| self.expression(argument))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(quote!(#target(#(#arguments),*)))
            }
            hir::Expression::AssociatedCall {
                rust_path,
                arguments,
            } => {
                let rust_path = path(rust_path)?;
                let arguments = arguments
                    .iter()
                    .map(|argument| self.expression(argument))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(quote!(#rust_path(#(#arguments),*)))
            }
            hir::Expression::MethodCall {
                receiver,
                rust_name,
                arguments,
                ..
            } => {
                let receiver = self.expression(receiver)?;
                let rust_name = identifier(rust_name)?;
                let arguments = arguments
                    .iter()
                    .map(|argument| self.expression(argument))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(quote!((#receiver).#rust_name(#(#arguments),*)))
            }
            hir::Expression::Clone { expression } => {
                let expression = self.expression(expression)?;
                Ok(quote!(::core::clone::Clone::clone(#expression)))
            }
            hir::Expression::Cast { expression, target } => {
                let expression = self.expression(expression)?;
                let target = type_tokens(target, None)?;
                Ok(quote!((#expression) as #target))
            }
            _ => unreachable!("only call-like expressions are delegated"),
        }
    }

    fn temporary(&mut self, purpose: &str) -> Result<syn::Ident, String> {
        let index = self.temporary_index;
        self.temporary_index += 1;
        identifier(&format!("__stainless_{purpose}_{index}"))
    }
}

fn identifier(name: &str) -> Result<syn::Ident, String> {
    syn::parse_str(name).map_err(|error| format!("invalid generated identifier `{name}`: {error}"))
}

fn parameter_tokens(
    parameter: &hir::Parameter,
    lifetime: Option<&syn::Lifetime>,
) -> Result<TokenStream, String> {
    let name = identifier(&parameter.rust_name)?;
    let ty = type_tokens(&parameter.ty, lifetime)?;
    let mutable = parameter.mutable.then(|| quote!(mut));
    Ok(quote!(#mutable #name: #ty))
}

fn type_tokens(ty: &hir::Type, lifetime: Option<&syn::Lifetime>) -> Result<TokenStream, String> {
    match ty {
        hir::Type::Unit => Ok(quote!(())),
        hir::Type::Primitive(name) => {
            let name = identifier(name)?;
            Ok(quote!(#name))
        }
        hir::Type::Native {
            rust_path,
            arguments,
        } => {
            let path = path(rust_path)?;
            let arguments = arguments
                .iter()
                .map(|argument| type_tokens(argument, None))
                .collect::<Result<Vec<_>, _>>()?;
            if arguments.is_empty() {
                Ok(quote!(#path))
            } else {
                Ok(quote!(#path < #(#arguments),* >))
            }
        }
        hir::Type::Reference { mutable, target } => {
            let target = type_tokens(target, None)?;
            let mutable = mutable.then(|| quote!(mut));
            Ok(quote!(& #lifetime #mutable #target))
        }
    }
}

fn path(source: &str) -> Result<syn::Path, String> {
    syn::parse_str(source).map_err(|error| format!("invalid generated path `{source}`: {error}"))
}

fn literal(kind: LiteralKind, text: &str) -> Result<TokenStream, String> {
    match kind {
        LiteralKind::String => {
            let literal = syn::parse_str::<syn::LitStr>(text)
                .map_err(|error| format!("invalid string literal `{text}`: {error}"))?;
            Ok(quote!(::std::string::String::from(#literal)))
        }
        LiteralKind::Character => {
            let literal = syn::parse_str::<syn::LitChar>(text)
                .map_err(|error| format!("invalid character literal `{text}`: {error}"))?;
            Ok(quote!(#literal))
        }
        LiteralKind::Integer => {
            let literal = syn::LitInt::new(text, TokenSpan::call_site());
            Ok(quote!(#literal))
        }
        LiteralKind::Float => {
            let normalized = text
                .strip_suffix('f')
                .map_or_else(|| text.to_owned(), |value| format!("{value}f32"));
            let literal = syn::LitFloat::new(&normalized, TokenSpan::call_site());
            Ok(quote!(#literal))
        }
        LiteralKind::Boolean => TokenStream::from_str(text)
            .map_err(|error| format!("invalid boolean literal `{text}`: {error}")),
    }
}

fn binary_operator(operator: BinaryOperator) -> TokenStream {
    match operator {
        BinaryOperator::Assign => quote!(=),
        BinaryOperator::AddAssign => quote!(+=),
        BinaryOperator::SubtractAssign => quote!(-=),
        BinaryOperator::MultiplyAssign => quote!(*=),
        BinaryOperator::DivideAssign => quote!(/=),
        BinaryOperator::RemainderAssign => quote!(%=),
        BinaryOperator::LogicalOr => quote!(||),
        BinaryOperator::LogicalAnd => quote!(&&),
        BinaryOperator::BitwiseOr => quote!(|),
        BinaryOperator::BitwiseXor => quote!(^),
        BinaryOperator::BitwiseAnd => quote!(&),
        BinaryOperator::Equal => quote!(==),
        BinaryOperator::NotEqual => quote!(!=),
        BinaryOperator::Less => quote!(<),
        BinaryOperator::LessEqual => quote!(<=),
        BinaryOperator::Greater => quote!(>),
        BinaryOperator::GreaterEqual => quote!(>=),
        BinaryOperator::Add => quote!(+),
        BinaryOperator::Subtract => quote!(-),
        BinaryOperator::Multiply => quote!(*),
        BinaryOperator::Divide => quote!(/),
        BinaryOperator::Remainder => quote!(%),
    }
}
