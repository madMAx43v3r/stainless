use std::str::FromStr;

use proc_macro2::{Ident, Span as TokenSpan, TokenStream};
use quote::quote;

use crate::ast::{BinaryOperator, LiteralKind, PrefixOperator};
use crate::hir;
use crate::interop::PointerKind;

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
        let exception_runtime = program_has_exceptions(program).then(|| {
            quote! {
                pub trait __StainlessException:
                    ::std::error::Error + ::std::any::Any
                {
                    fn __stainless_project(
                        &self,
                        target: ::std::any::TypeId,
                    ) -> Option<&dyn ::std::any::Any>;
                    fn __stainless_message(&self) -> &str;
                }

                pub type __StainlessExceptionBox = Box<dyn __StainlessException>;
            }
        });
        let structs = program
            .structs
            .iter()
            .map(Self::structure)
            .collect::<Result<Vec<_>, _>>()?;
        let interfaces = program
            .interfaces
            .iter()
            .map(Self::interface)
            .collect::<Result<Vec<_>, _>>()?;
        let native_wrappers = program
            .native_wrappers
            .iter()
            .map(Self::native_wrapper)
            .collect::<Result<Vec<_>, _>>()?;
        let native_wrapper_module = (!native_wrappers.is_empty()).then(|| {
            quote! {
                #[allow(non_snake_case)]
                mod __stainless_bindings {
                    #(#native_wrappers)*
                }
            }
        });
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
            #exception_runtime
            #native_wrapper_module
            #(#interfaces)*
            #(#structs)*
            #(#functions)*
            #(#modules)*
        })
    }

    #[allow(clippy::too_many_lines)]
    fn native_wrapper(wrapper: &hir::NativeWrapper) -> Result<TokenStream, String> {
        let name = identifier(&wrapper.rust_name)?;
        let mut parameter_declarations = Vec::new();
        let mut call_arguments = Vec::new();
        let mut generic_parameters = Vec::new();
        let mut callback_bounds = Vec::new();
        let receiver_name = identifier("__stainless_receiver")?;
        if let Some(receiver) = &wrapper.receiver {
            let receiver_type = type_tokens(&receiver.ty, None)?;
            let receiver_type = match receiver.mode {
                crate::interop::Receiver::Shared => quote!(&#receiver_type),
                crate::interop::Receiver::Mutable => quote!(&mut #receiver_type),
                crate::interop::Receiver::Value => quote!(#receiver_type),
            };
            parameter_declarations.push(quote!(#receiver_name: #receiver_type));
        }
        for (index, parameter) in wrapper.parameters.iter().enumerate() {
            let parameter_name = identifier(&format!("__stainless_argument_{index}"))?;
            let parameter_type = match &parameter.ty {
                hir::Type::Callback {
                    is_async,
                    kind,
                    escape,
                    parameters,
                    return_type,
                } => {
                    if parameter.adaptation != crate::interop::ArgumentAdaptation::Identity {
                        return Err(format!(
                            "generated callback parameter {index} has an unsupported adaptation"
                        ));
                    }
                    let callback_parameters = parameters
                        .iter()
                        .map(|parameter| type_tokens(parameter, None))
                        .collect::<Result<Vec<_>, _>>()?;
                    let callback_return = type_tokens(return_type, None)?;
                    if *kind == crate::interop::CallbackKind::FunctionPointer {
                        quote!(fn(#(#callback_parameters),*) -> #callback_return)
                    } else {
                        let generic = identifier(&format!("__StainlessCallback{index}"))?;
                        let trait_path = match kind {
                            crate::interop::CallbackKind::Fn => quote!(::core::ops::Fn),
                            crate::interop::CallbackKind::FnMut => {
                                quote!(::core::ops::FnMut)
                            }
                            crate::interop::CallbackKind::FnOnce => {
                                quote!(::core::ops::FnOnce)
                            }
                            crate::interop::CallbackKind::FunctionPointer => {
                                unreachable!("function pointers do not use callback generics")
                            }
                        };
                        generic_parameters.push(quote!(#generic));
                        let thread_bounds = (*escape == crate::interop::CallbackEscape::Thread)
                            .then(|| {
                                if *is_async && *kind == crate::interop::CallbackKind::Fn {
                                    quote!(+ ::core::marker::Send + ::core::marker::Sync + 'static)
                                } else {
                                    quote!(+ ::core::marker::Send + 'static)
                                }
                            });
                        if *is_async {
                            let future = identifier(&format!("__StainlessFuture{index}"))?;
                            generic_parameters.push(quote!(#future));
                            callback_bounds.push(quote!(
                                #generic: #trait_path(#(#callback_parameters),*)
                                    -> #future #thread_bounds
                            ));
                            let future_thread_bounds = (*escape
                                == crate::interop::CallbackEscape::Thread)
                                .then(|| quote!(+ ::core::marker::Send + 'static));
                            callback_bounds.push(quote!(
                                #future: ::core::future::Future<Output = #callback_return>
                                    #future_thread_bounds
                            ));
                        } else {
                            callback_bounds.push(quote!(
                                #generic: #trait_path(#(#callback_parameters),*)
                                    -> #callback_return #thread_bounds
                            ));
                        }
                        quote!(#generic)
                    }
                }
                _ => type_tokens(&parameter.ty, None)?,
            };
            parameter_declarations.push(quote!(#parameter_name: #parameter_type));
            let argument = match parameter.adaptation {
                crate::interop::ArgumentAdaptation::Identity => quote!(#parameter_name),
                crate::interop::ArgumentAdaptation::StringRefToStr => {
                    quote!((#parameter_name).as_str())
                }
            };
            call_arguments.push(argument);
        }
        let return_type = type_tokens(&wrapper.return_type, None)?;
        let call = match &wrapper.target {
            crate::interop::WrapperTarget::Function { rust_path } => {
                let target = path(rust_path)?;
                quote!(#target(#(#call_arguments),*))
            }
            crate::interop::WrapperTarget::Method { rust_name } => {
                if wrapper.receiver.is_none() {
                    return Err(format!(
                        "generated method wrapper `{}` has no receiver",
                        wrapper.rust_name
                    ));
                }
                let method = identifier(rust_name)?;
                quote!((#receiver_name).#method(#(#call_arguments),*))
            }
        };
        let call = if wrapper.is_async {
            quote!((#call).await)
        } else {
            call
        };
        let call = match wrapper.return_adaptation {
            crate::interop::ReturnAdaptation::Identity => call,
            crate::interop::ReturnAdaptation::Into => quote!((#call).into()),
        };
        let generics = (!generic_parameters.is_empty()).then(|| quote!(<#(#generic_parameters),*>));
        let where_clause =
            (!callback_bounds.is_empty()).then(|| quote!(where #(#callback_bounds),*));
        let asyncness = wrapper.is_async.then(|| quote!(async));
        Ok(quote! {
            #[allow(deprecated, non_snake_case)]
            pub(crate) #asyncness fn #name #generics (
                #(#parameter_declarations),*
            ) -> #return_type
            #where_clause
            {
                #call
            }
        })
    }

    fn module(&mut self, module: &hir::Module) -> Result<TokenStream, String> {
        let name = identifier(&module.rust_name)?;
        let interfaces = module
            .interfaces
            .iter()
            .map(Self::interface)
            .collect::<Result<Vec<_>, _>>()?;
        let structs = module
            .structs
            .iter()
            .map(Self::structure)
            .collect::<Result<Vec<_>, _>>()?;
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
                #(#interfaces)*
                #(#structs)*
                #(#functions)*
                #(#modules)*
            }
        })
    }

    fn structure(structure: &hir::Struct) -> Result<TokenStream, String> {
        let name = identifier(&structure.rust_name)?;
        let type_parameters = generic_parameter_declarations(
            &structure.type_parameters,
            &structure.const_parameters,
            false,
        )?;
        let generics = if type_parameters.is_empty() {
            TokenStream::new()
        } else {
            quote!(<#(#type_parameters),*>)
        };
        let fields = Self::structure_fields(&structure.fields)?;
        let associated_constants =
            Self::structure_static_constants(&name, &generics, &structure.static_constants)?;
        let json_conversion = Self::struct_json_conversion(
            &name,
            &structure.source_path,
            structure.json_fields.as_deref(),
        )?;
        let exception_impl = if structure.is_exception {
            let projection_fallback = if let Some(base_field) = &structure.exception_base_field {
                let base_field = identifier(base_field)?;
                quote! {
                    crate::__StainlessException::__stainless_project(
                        &self.#base_field,
                        target,
                    )
                }
            } else {
                quote!(None)
            };
            let message = if let Some(base_field) = &structure.exception_base_field {
                let base_field = identifier(base_field)?;
                quote! {
                    crate::__StainlessException::__stainless_message(&self.#base_field)
                }
            } else {
                let message = identifier("message")?;
                quote!((&self.#message).as_str())
            };
            quote! {
                impl crate::__StainlessException for #name {
                    fn __stainless_project(
                        &self,
                        target: ::std::any::TypeId,
                    ) -> Option<&dyn ::std::any::Any> {
                        if target == ::std::any::TypeId::of::<Self>() {
                            Some(self)
                        } else {
                            #projection_fallback
                        }
                    }

                    fn __stainless_message(&self) -> &str {
                        #message
                    }
                }

                impl ::std::fmt::Display for #name {
                    fn fmt(
                        &self,
                        formatter: &mut ::std::fmt::Formatter<'_>,
                    ) -> ::std::fmt::Result {
                        formatter.write_str(
                            crate::__StainlessException::__stainless_message(self),
                        )
                    }
                }

                impl ::std::fmt::Debug for #name {
                    fn fmt(
                        &self,
                        formatter: &mut ::std::fmt::Formatter<'_>,
                    ) -> ::std::fmt::Result {
                        ::std::fmt::Display::fmt(self, formatter)
                    }
                }

                impl ::std::error::Error for #name {}
            }
        } else {
            TokenStream::new()
        };
        let derive = structure.copyable.then(|| quote!(#[derive(Clone)]));
        let interface_implementations = structure
            .interface_implementations
            .iter()
            .map(|implementation| Self::interface_implementation(&name, implementation))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(quote! {
            #derive
            #[allow(non_snake_case)]
            pub struct #name #generics {
                #(#fields),*
            }
            #associated_constants
            #json_conversion
            #exception_impl
            #(#interface_implementations)*
        })
    }

    fn structure_static_constants(
        structure: &Ident,
        generics: &TokenStream,
        constants: &[hir::StaticConstant],
    ) -> Result<TokenStream, String> {
        let constants = constants
            .iter()
            .map(|constant| {
                let name = identifier(&constant.rust_name)?;
                let visibility = constant.is_public.then(|| quote!(pub));
                let ty = type_tokens(&constant.ty, None)?;
                let value = literal(LiteralKind::Integer, &constant.value)?;
                Ok(quote!(#visibility const #name: #ty = #value;))
            })
            .collect::<Result<Vec<_>, String>>()?;
        if constants.is_empty() {
            return Ok(TokenStream::new());
        }
        Ok(quote! {
            #[allow(non_upper_case_globals)]
            impl #generics #structure #generics {
                #(#constants)*
            }
        })
    }

    fn structure_fields(fields: &[hir::Field]) -> Result<Vec<TokenStream>, String> {
        fields
            .iter()
            .map(|field| {
                let name = identifier(&field.rust_name)?;
                let ty = type_tokens(&field.ty, None)?;
                Ok(quote!(pub #name: #ty))
            })
            .collect()
    }

    fn struct_json_conversion(
        structure_name: &Ident,
        source_path: &[String],
        fields: Option<&[hir::JsonStructField]>,
    ) -> Result<TokenStream, String> {
        let Some(fields) = fields else {
            return Ok(TokenStream::new());
        };
        let value = identifier("__stainless_json_value")?;
        let source_type = source_path.join("::");
        let field_members = fields
            .iter()
            .map(|field| {
                let mut access = quote!(#value);
                for segment in &field.access_path {
                    let segment = identifier(segment)?;
                    access = quote!((#access).#segment);
                }
                let member_name = &field.name;
                Ok(quote!((
                    #member_name,
                    ::stainless_runtime::Var::from(#access),
                )))
            })
            .collect::<Result<Vec<_>, String>>()?;
        Ok(quote! {
            impl ::core::convert::From<#structure_name> for ::stainless_runtime::Var {
                fn from(#value: #structure_name) -> Self {
                    ::stainless_runtime::Var::object([
                        (
                            "__type",
                            ::stainless_runtime::Var::from(#source_type),
                        ),
                        #(#field_members),*
                    ])
                }
            }
        })
    }

    fn interface(interface: &hir::Interface) -> Result<TokenStream, String> {
        let name = identifier(&interface.rust_name)?;
        let bases = interface
            .bases
            .iter()
            .map(|base| path(base))
            .collect::<Result<Vec<_>, _>>()?;
        let supertraits = if bases.is_empty() {
            TokenStream::new()
        } else {
            quote!(: #(#bases)+*)
        };
        let methods = interface
            .methods
            .iter()
            .map(|method| Self::interface_method_signature(method, true))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(quote! {
            #[allow(non_snake_case)]
            pub trait #name #supertraits {
                #(#methods)*
            }
        })
    }

    fn interface_implementation(
        concrete: &syn::Ident,
        implementation: &hir::InterfaceImplementation,
    ) -> Result<TokenStream, String> {
        let interface = path(&implementation.interface_path)?;
        let methods = implementation
            .methods
            .iter()
            .map(|implementation| {
                let signature = Self::interface_method_signature(&implementation.method, false)?;
                let target = if implementation.function_modules.is_empty() {
                    format!("crate::{}", implementation.function)
                } else {
                    format!(
                        "crate::{}::{}",
                        implementation.function_modules.join("::"),
                        implementation.function
                    )
                };
                let target = path(&target)?;
                let arguments = implementation
                    .method
                    .parameters
                    .iter()
                    .map(|parameter| identifier(&parameter.rust_name))
                    .collect::<Result<Vec<_>, _>>()?;
                let invocation = quote!(#target(self, #(#arguments),*));
                let invocation = if implementation.adapt_self_reference {
                    let return_type = type_tokens(&implementation.method.return_type, None)?;
                    if implementation.method.throws {
                        quote!((#invocation).map(|value| value as #return_type))
                    } else {
                        quote!((#invocation) as #return_type)
                    }
                } else {
                    invocation
                };
                Ok(quote! {
                    #signature {
                        #invocation
                    }
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        Ok(quote! {
            impl #interface for #concrete {
                #(#methods)*
            }
        })
    }

    fn interface_method_signature(
        method: &hir::InterfaceMethod,
        declaration: bool,
    ) -> Result<TokenStream, String> {
        let name = identifier(&method.rust_name)?;
        let receiver = if method.mutable {
            quote!(&mut self)
        } else {
            quote!(&self)
        };
        let parameters = method
            .parameters
            .iter()
            .map(|parameter| parameter_tokens(parameter, None))
            .collect::<Result<Vec<_>, _>>()?;
        let return_type = type_tokens(&method.return_type, None)?;
        let return_signature = if method.throws {
            quote!(-> ::std::result::Result<#return_type, crate::__StainlessExceptionBox>)
        } else if matches!(method.return_type, hir::Type::Unit) {
            TokenStream::new()
        } else {
            quote!(-> #return_type)
        };
        let semicolon = declaration.then(|| quote!(;));
        Ok(quote!(fn #name(#receiver, #(#parameters),*) #return_signature #semicolon))
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
        let return_signature = if function.throws {
            quote!(-> ::std::result::Result<#return_type, crate::__StainlessExceptionBox>)
        } else if matches!(function.return_type, hir::Type::Unit) {
            TokenStream::new()
        } else {
            quote!(-> #return_type)
        };
        let body = self.block(&function.body)?;
        let generic_parameters = generic_parameter_declarations(
            &function.type_parameters,
            &function.const_parameters,
            true,
        )?;
        // Stainless generic arguments are owned value types: references cannot
        // be stored in fields or nested inside a generic argument. Reflect that
        // invariant in Rust so a generic value may safely appear in an escaping
        // `function` / `function_mut` closure.
        let generics = match (explicit_lifetime, generic_parameters.is_empty()) {
            (false, true) => None,
            (true, true) => Some(quote!(<'__stainless_borrow>)),
            (false, false) => Some(quote!(<#(#generic_parameters),*>)),
            (true, false) => Some(quote!(<'__stainless_borrow, #(#generic_parameters),*>)),
        };
        let asyncness = function.is_async.then(|| quote!(async));
        Ok(quote! {
            #[allow(
                non_snake_case,
                unreachable_code,
                unused_mut,
                unused_parens,
                unused_variables,
            )]
            pub #asyncness fn #name #generics (#(#parameters),*) #return_signature #body
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

    #[allow(clippy::too_many_lines)]
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
            hir::Statement::Throw { value, target } => self.throw_statement(value, target),
            statement @ hir::Statement::Try { .. } => self.try_statement(statement),
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
            hir::Statement::While {
                label,
                condition,
                body,
            } => {
                let label = rust_label(label);
                let condition = self.expression(condition)?;
                let body = self.block(body)?;
                Ok(quote!(#label: while #condition #body))
            }
            hir::Statement::ClassicFor {
                label,
                initializer,
                condition,
                update,
                body,
            } => self.classic_for(
                label,
                initializer.as_deref(),
                condition.as_ref(),
                update.as_ref(),
                body,
            ),
            hir::Statement::RangeFor {
                label,
                bindings,
                mode,
                iterable,
                body,
            } => {
                let label = rust_label(label);
                let bindings = bindings
                    .iter()
                    .map(|binding| {
                        let name = identifier(&binding.name)?;
                        let mutable = binding.mutable.then(|| quote!(mut));
                        Ok(quote!(#mutable #name))
                    })
                    .collect::<Result<Vec<_>, String>>()?;
                let pattern = match bindings.as_slice() {
                    [binding] => binding.clone(),
                    [key, value] => quote!((#key, #value)),
                    _ => return Err("range loop requires one or two bindings".to_owned()),
                };
                let iterable = self.expression(iterable)?;
                let iterator = match mode {
                    hir::RangeMode::Shared => quote!((#iterable).iter()),
                    hir::RangeMode::Mutable => quote!((#iterable).iter_mut()),
                    hir::RangeMode::Copy => quote!((#iterable).iter().copied()),
                    hir::RangeMode::Clone => quote!((#iterable).iter().cloned()),
                    hir::RangeMode::MapClone => quote!(
                        (#iterable)
                            .iter()
                            .map(|(key, value)| ((*key).clone(), (*value).clone()))
                    ),
                    hir::RangeMode::Move => quote!((#iterable).into_iter()),
                };
                let body = self.block(body)?;
                Ok(quote!(#label: for #pattern in #iterator #body))
            }
            hir::Statement::Break(label) => {
                let label = rust_label(label);
                Ok(quote!(break #label;))
            }
            hir::Statement::Continue(label) => {
                let label = rust_label(label);
                Ok(quote!(continue #label;))
            }
            hir::Statement::Expression(expression) => {
                let expression = self.expression(expression)?;
                Ok(quote!(#expression;))
            }
        }
    }

    fn throw_statement(
        &mut self,
        value: &hir::ExceptionValue,
        target: &hir::ExceptionTarget,
    ) -> Result<TokenStream, String> {
        let error = match value {
            hir::ExceptionValue::New(value) => {
                let value = self.expression(value)?;
                quote! {
                    Box::new(#value) as crate::__StainlessExceptionBox
                }
            }
            hir::ExceptionValue::Existing(name) => {
                let name = identifier(name)?;
                quote!(#name)
            }
        };
        Ok(exception_propagation(target, &error))
    }

    fn try_statement(&mut self, statement: &hir::Statement) -> Result<TokenStream, String> {
        let hir::Statement::Try {
            label,
            error_name,
            body,
            body_falls_through,
            catches,
            diverges,
            unmatched_target,
        } = statement
        else {
            return Err("internal try emitter received a non-try statement".to_owned());
        };
        let label = syn::Lifetime::new(&format!("'{label}"), TokenSpan::call_site());
        let result_name = self.temporary("try_result")?;
        let error_name = identifier(error_name)?;
        let body = self.block(body)?;
        let unmatched_error = quote!(#error_name);
        let unmatched = exception_propagation(unmatched_target, &unmatched_error);
        let mut handlers = unmatched;
        for catch in catches.iter().rev() {
            let catch_body = self.block(&catch.body)?;
            if let Some(ty) = &catch.ty {
                let ty = type_tokens(ty, None)?;
                let binding = catch
                    .binding
                    .as_deref()
                    .ok_or_else(|| "typed catch is missing its binding".to_owned())
                    .and_then(identifier)?;
                handlers = quote! {
                    if let Some(#binding) =
                        crate::__StainlessException::__stainless_project(
                            &*#error_name,
                            ::std::any::TypeId::of::<#ty>(),
                        )
                        .and_then(|value| value.downcast_ref::<#ty>())
                    {
                        #catch_body
                    } else {
                        #handlers
                    }
                };
            } else {
                handlers = quote! { #catch_body };
            }
        }
        let normal_completion = body_falls_through.then(|| quote!(break #label Ok(());));
        let dispatch = if *diverges {
            quote! {
                match #result_name {
                    Ok(()) => unreachable!(
                        "statically diverging Stainless try completed normally"
                    ),
                    Err(#error_name) => {
                        #handlers
                    }
                }
            }
        } else {
            quote! {
                if let Err(#error_name) = #result_name {
                    #handlers
                }
            }
        };
        Ok(quote! {
            let #result_name: ::std::result::Result<
                (),
                crate::__StainlessExceptionBox,
            > = #label: {
                #body
                #normal_completion
            };
            #dispatch
        })
    }

    fn classic_for(
        &mut self,
        label: &str,
        initializer: Option<&hir::ForInitializer>,
        condition: Option<&hir::Expression>,
        update: Option<&hir::Expression>,
        body: &hir::Block,
    ) -> Result<TokenStream, String> {
        let label = rust_label(label);
        let initializer = initializer
            .map(|initializer| self.for_initializer(initializer))
            .transpose()?;
        let condition = condition
            .map(|condition| self.expression(condition))
            .transpose()?;
        let condition_check = condition.map(|condition| {
            quote! {
                if !(#condition) {
                    break #label;
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
                #label: loop {
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
                #label: loop {
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

    #[allow(clippy::too_many_lines)]
    fn expression(&mut self, expression: &hir::Expression) -> Result<TokenStream, String> {
        match expression {
            hir::Expression::Tuple(elements) => {
                let elements = elements
                    .iter()
                    .map(|element| self.expression(element))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(quote!((#(#elements),*)))
            }
            hir::Expression::Array { elements, default } => {
                let elements = elements
                    .iter()
                    .map(|element| self.expression(element))
                    .collect::<Result<Vec<_>, _>>()?;
                let Some(default) = default else {
                    return Ok(quote!([#(#elements),*]));
                };
                let default = self.expression(default)?;
                if elements.is_empty() {
                    Ok(quote!(::core::array::from_fn(|_| #default)))
                } else {
                    let values = self.temporary("array_values")?;
                    Ok(quote!({
                        let mut #values = [#(#elements),*].into_iter();
                        ::core::array::from_fn(|_| match #values.next() {
                            ::core::option::Option::Some(value) => value,
                            ::core::option::Option::None => #default,
                        })
                    }))
                }
            }
            hir::Expression::DefaultValue(ty) => {
                let ty = type_tokens(ty, None)?;
                Ok(quote!(<#ty as ::core::default::Default>::default()))
            }
            hir::Expression::Name(name) => {
                let name = identifier(name)?;
                Ok(quote!(#name))
            }
            hir::Expression::StaticConstant {
                modules,
                structure,
                constant,
            } => {
                let target = if modules.is_empty() {
                    format!("crate::{structure}")
                } else {
                    format!("crate::{}::{structure}", modules.join("::"))
                };
                let target = path(&target)?;
                let constant = identifier(constant)?;
                Ok(quote!(#target::#constant))
            }
            hir::Expression::Literal { kind, text } => literal(*kind, text),
            hir::Expression::Switch {
                scrutinee,
                arms,
                string_scrutinee,
            } => {
                let scrutinee = self.expression(scrutinee)?;
                let scrutinee = if *string_scrutinee {
                    quote!((#scrutinee).as_str())
                } else {
                    scrutinee
                };
                let arms = arms
                    .iter()
                    .map(|arm| {
                        let pattern = match &arm.pattern {
                            hir::SwitchPattern::Literals(literals) => {
                                let alternatives = literals
                                    .iter()
                                    .map(|literal| switch_literal(literal.kind, &literal.text))
                                    .collect::<Result<Vec<_>, String>>()?;
                                quote!(#(#alternatives)|*)
                            }
                            hir::SwitchPattern::Fallback => quote!(_),
                        };
                        let value = self.expression(&arm.value)?;
                        Ok(quote!(#pattern => #value))
                    })
                    .collect::<Result<Vec<_>, String>>()?;
                Ok(quote!(match #scrutinee { #(#arms),* }))
            }
            hir::Expression::JsonNull => Ok(quote!(::stainless_runtime::Var::null())),
            hir::Expression::JsonArray(elements) => {
                let elements = elements
                    .iter()
                    .map(|element| self.expression(element))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(quote!(::stainless_runtime::Var::array([#(#elements),*])))
            }
            hir::Expression::JsonObject(members) => {
                if members.is_empty() {
                    return Ok(quote!(::stainless_runtime::Var::empty_object()));
                }
                let members = members
                    .iter()
                    .map(|(name, value)| {
                        let value = self.expression(value)?;
                        Ok(quote!((#name, #value)))
                    })
                    .collect::<Result<Vec<_>, String>>()?;
                Ok(quote!(::stainless_runtime::Var::object([#(#members),*])))
            }
            hir::Expression::JsonFrom(expression) => {
                let expression = self.expression(expression)?;
                Ok(quote!(::stainless_runtime::Var::from(#expression)))
            }
            hir::Expression::JsonField { receiver, name } => {
                let receiver = self.expression(receiver)?;
                Ok(quote!((#receiver).field(#name)))
            }
            hir::Expression::JsonIndex { receiver, index } => {
                let receiver = self.expression(receiver)?;
                let index = checked_index(&self.expression(index)?);
                Ok(quote!((#receiver).index(#index)))
            }
            hir::Expression::SequenceIndex { receiver, index } => {
                let receiver = self.expression(receiver)?;
                let index = checked_index(&self.expression(index)?);
                Ok(quote!((#receiver)[#index]))
            }
            hir::Expression::JsonSetField {
                receiver,
                name,
                value,
            } => {
                let receiver = self.expression(receiver)?;
                let value = self.expression(value)?;
                Ok(quote!((#receiver).set_field(#name, #value)))
            }
            hir::Expression::JsonSetIndex {
                receiver,
                index,
                value,
            } => {
                let receiver = self.expression(receiver)?;
                let index = checked_index(&self.expression(index)?);
                let value = self.expression(value)?;
                Ok(quote!((#receiver).set_index(#index, #value)))
            }
            hir::Expression::JsonCast { expression, target } => {
                let expression = self.expression(expression)?;
                let method = identifier(json_cast_method(target)?)?;
                Ok(quote!((#expression).#method()))
            }
            hir::Expression::FormatMacro {
                kind,
                destination,
                format,
                arguments,
            } => {
                let arguments = arguments
                    .iter()
                    .map(|argument| self.expression(argument))
                    .collect::<Result<Vec<_>, _>>()?;
                let format = format
                    .as_deref()
                    .map(syn::parse_str::<syn::LitStr>)
                    .transpose()
                    .map_err(|error| format!("invalid formatting macro literal: {error}"))?;
                match kind {
                    hir::FormatMacroKind::Println => match format {
                        Some(format) => Ok(quote!(::std::println!(#format #(, #arguments)*))),
                        None => Ok(quote!(::std::println!())),
                    },
                    hir::FormatMacroKind::Eprintln => match format {
                        Some(format) => Ok(quote!(::std::eprintln!(#format #(, #arguments)*))),
                        None => Ok(quote!(::std::eprintln!())),
                    },
                    hir::FormatMacroKind::Format => {
                        let format = format
                            .ok_or_else(|| "resolved `format!` has no format literal".to_owned())?;
                        Ok(quote!(::std::format!(#format #(, #arguments)*)))
                    }
                    hir::FormatMacroKind::Write | hir::FormatMacroKind::Writeln => {
                        let destination = destination
                            .as_deref()
                            .ok_or_else(|| "resolved write macro has no destination".to_owned())?;
                        let destination = self.expression(destination)?;
                        let invocation = match (kind, format) {
                            (hir::FormatMacroKind::Write, Some(format)) => {
                                quote!(::std::write!(#destination, #format #(, #arguments)*))
                            }
                            (hir::FormatMacroKind::Writeln, Some(format)) => {
                                quote!(::std::writeln!(#destination, #format #(, #arguments)*))
                            }
                            (hir::FormatMacroKind::Writeln, None) => {
                                quote!(::std::writeln!(#destination))
                            }
                            (hir::FormatMacroKind::Write, None) => {
                                return Err("resolved `write!` has no format literal".to_owned());
                            }
                            _ => unreachable!("non-write macro handled above"),
                        };
                        Ok(quote!({
                            use ::std::fmt::Write as _;
                            #invocation
                        }))
                    }
                }
            }
            hir::Expression::Parenthesized(expression) => {
                let expression = self.expression(expression)?;
                Ok(quote!((#expression)))
            }
            hir::Expression::Borrow {
                mutable,
                expression,
            } => {
                let expression = self.expression(expression)?;
                if *mutable {
                    Ok(quote!(&mut (#expression)))
                } else {
                    Ok(quote!(&(#expression)))
                }
            }
            hir::Expression::Dereference(expression) => {
                let expression = self.expression(expression)?;
                Ok(quote!(*(#expression)))
            }
            hir::Expression::Move(expression) => {
                let expression = self.expression(expression)?;
                let temporary = self.temporary("moved")?;
                Ok(quote!({
                    let #temporary = #expression;
                    #temporary
                }))
            }
            hir::Expression::MakeOwner { kind, value } => {
                let value = self.expression(value)?;
                match kind {
                    crate::interop::PointerKind::Unique => {
                        Ok(quote!(::std::boxed::Box::new(#value)))
                    }
                    crate::interop::PointerKind::Shared => {
                        Ok(quote!(::std::sync::Arc::new(#value)))
                    }
                    _ => Err(
                        "nullable, weak, or atomic owner reached allocation lowering".to_owned(),
                    ),
                }
            }
            hir::Expression::PointerDefault(kind) => match kind {
                crate::interop::PointerKind::UniqueNullable
                | crate::interop::PointerKind::SharedNullable => {
                    Ok(quote!(::core::option::Option::None))
                }
                crate::interop::PointerKind::Weak => Ok(quote!(::std::sync::Weak::new())),
                crate::interop::PointerKind::AtomicNullable => Ok(quote!(
                    ::std::sync::RwLock::new(::core::option::Option::None)
                )),
                _ => Err("non-default-constructible pointer reached code generation".to_owned()),
            },
            hir::Expression::PointerConversion { from, to, value } => {
                let value = self.expression(value)?;
                match (from, to) {
                    (PointerKind::Unique, PointerKind::UniqueNullable)
                    | (PointerKind::Shared, PointerKind::SharedNullable) => {
                        Ok(quote!(::core::option::Option::Some(#value)))
                    }
                    (PointerKind::Shared, PointerKind::Atomic)
                    | (PointerKind::SharedNullable, PointerKind::AtomicNullable) => {
                        Ok(quote!(::std::sync::RwLock::new(#value)))
                    }
                    (PointerKind::Shared, PointerKind::AtomicNullable) => Ok(quote!(
                        ::std::sync::RwLock::new(::core::option::Option::Some(#value))
                    )),
                    (PointerKind::UniqueNullable, PointerKind::Unique)
                    | (PointerKind::SharedNullable, PointerKind::Shared) => {
                        Ok(quote!(match #value {
                            ::core::option::Option::Some(value) => value,
                            ::core::option::Option::None => ::core::unreachable!(
                                "Stainless non-null refinement was violated"
                            ),
                        }))
                    }
                    _ => Err("unsupported pointer conversion reached code generation".to_owned()),
                }
            }
            hir::Expression::InterfaceOwnerCoercion {
                kind,
                target,
                value,
            } => {
                let value = self.expression(value)?;
                let target = type_tokens(target, None)?;
                match kind {
                    PointerKind::Unique => Ok(quote!((#value) as ::std::boxed::Box<#target>)),
                    PointerKind::UniqueNullable => Ok(quote!((#value).map(|value| {
                        value as ::std::boxed::Box<#target>
                    }))),
                    PointerKind::Shared => Ok(quote!((#value) as ::std::sync::Arc<#target>)),
                    PointerKind::SharedNullable => Ok(quote!((#value).map(|value| {
                        value as ::std::sync::Arc<#target>
                    }))),
                    _ => Err("non-owning pointer reached interface owner coercion".to_owned()),
                }
            }
            hir::Expression::ClassSharedOwnerCoercion {
                projection,
                nullable,
                value,
            } => {
                let value = self.expression(value)?;
                if *nullable {
                    let derived = self.temporary("derived_class_owner")?;
                    let mut base = quote!(#derived);
                    for field in projection {
                        let field = identifier(field)?;
                        base = quote!((#base).#field);
                    }
                    Ok(quote!((#value).as_ref().map(|#derived| {
                        ::stainless_runtime::ClassBase::share(&(#base))
                    })))
                } else {
                    let mut base = value;
                    for field in projection {
                        let field = identifier(field)?;
                        base = quote!((#base).#field);
                    }
                    Ok(quote!(::stainless_runtime::ClassBase::share(&(#base))))
                }
            }
            hir::Expression::ClassBaseNew(value) => {
                let value = self.expression(value)?;
                Ok(quote!(::stainless_runtime::ClassBase::new(#value)))
            }
            hir::Expression::DowngradeShared(value) => {
                let value = self.expression(value)?;
                Ok(quote!(::std::sync::Arc::downgrade(&(#value))))
            }
            hir::Expression::LockWeak(value) => {
                let value = self.expression(value)?;
                Ok(quote!(::std::sync::Weak::upgrade(&(#value))))
            }
            hir::Expression::PointerHasValue { kind, value } => {
                let value = self.expression(value)?;
                match kind {
                    crate::interop::PointerKind::UniqueNullable
                    | crate::interop::PointerKind::SharedNullable => Ok(quote!((#value).is_some())),
                    crate::interop::PointerKind::Weak => Ok(quote!((#value).strong_count() != 0)),
                    _ => Err("non-nullable pointer reached null-test lowering".to_owned()),
                }
            }
            hir::Expression::PointerPointee {
                kind,
                mutable,
                owner,
            } => {
                let owner = self.expression(owner)?;
                match (kind, mutable) {
                    (crate::interop::PointerKind::Unique, true)
                    | (
                        crate::interop::PointerKind::Unique | crate::interop::PointerKind::Shared,
                        false,
                    ) => Ok(quote!(*(#owner))),
                    (crate::interop::PointerKind::UniqueNullable, true) => {
                        Ok(quote!(*(#owner).as_mut().expect(
                            "Stainless non-null refinement was violated"
                        )))
                    }
                    (
                        crate::interop::PointerKind::UniqueNullable
                        | crate::interop::PointerKind::SharedNullable,
                        false,
                    ) => Ok(quote!(*(#owner).as_ref().expect(
                        "Stainless non-null refinement was violated"
                    ))),
                    _ => Err(
                        "invalid nullable pointer projection reached code generation".to_owned(),
                    ),
                }
            }
            hir::Expression::AtomicLoad { slot, .. } => {
                let slot = self.expression(slot)?;
                let poisoned = self.temporary("poisoned_atomic_pointer")?;
                Ok(quote!(match (#slot).read() {
                    Ok(guard) => ::core::clone::Clone::clone(&*guard),
                    Err(#poisoned) => {
                        ::core::clone::Clone::clone(&*#poisoned.into_inner())
                    }
                }))
            }
            hir::Expression::AtomicStore { slot, value } => {
                let slot = self.expression(slot)?;
                let value = self.expression(value)?;
                let replacement = self.temporary("atomic_pointer_replacement")?;
                let poisoned = self.temporary("poisoned_atomic_pointer")?;
                Ok(quote!({
                    let #replacement = #value;
                    match (#slot).write() {
                        Ok(mut guard) => { *guard = #replacement; }
                        Err(#poisoned) => { *#poisoned.into_inner() = #replacement; }
                    }
                }))
            }
            hir::Expression::AtomicSwap { slot, value } => {
                let slot = self.expression(slot)?;
                let value = self.expression(value)?;
                let replacement = self.temporary("atomic_pointer_replacement")?;
                let poisoned = self.temporary("poisoned_atomic_pointer")?;
                Ok(quote!({
                    let #replacement = #value;
                    match (#slot).write() {
                        Ok(mut guard) => ::core::mem::replace(&mut *guard, #replacement),
                        Err(#poisoned) => ::core::mem::replace(
                            &mut *#poisoned.into_inner(),
                            #replacement,
                        ),
                    }
                }))
            }
            hir::Expression::MutexNew(value) => {
                let value = self.expression(value)?;
                Ok(quote!(::std::sync::Mutex::new(#value)))
            }
            hir::Expression::ConditionNew => Ok(quote!(::std::sync::Condvar::new())),
            hir::Expression::MutexLock(mutex) => {
                let mutex = self.expression(mutex)?;
                let poisoned = self.temporary("poisoned_mutex")?;
                Ok(quote!(match (#mutex).lock() {
                    Ok(guard) => guard,
                    Err(#poisoned) => #poisoned.into_inner(),
                }))
            }
            hir::Expression::RwLockNew(value) => {
                let value = self.expression(value)?;
                Ok(quote!(::std::sync::RwLock::new(#value)))
            }
            hir::Expression::RwLockRead(lock) => {
                let lock = self.expression(lock)?;
                let poisoned = self.temporary("poisoned_rwlock_read")?;
                Ok(quote!(match (#lock).read() {
                    Ok(guard) => guard,
                    Err(#poisoned) => #poisoned.into_inner(),
                }))
            }
            hir::Expression::RwLockWrite(lock) => {
                let lock = self.expression(lock)?;
                let poisoned = self.temporary("poisoned_rwlock_write")?;
                Ok(quote!(match (#lock).write() {
                    Ok(guard) => guard,
                    Err(#poisoned) => #poisoned.into_inner(),
                }))
            }
            hir::Expression::ConditionWait { condition, guard } => {
                let condition = self.expression(condition)?;
                let guard = self.expression(guard)?;
                let poisoned = self.temporary("poisoned_mutex_wait")?;
                Ok(quote!({
                    #guard = match (#condition).wait(#guard) {
                        Ok(next_guard) => next_guard,
                        Err(#poisoned) => #poisoned.into_inner(),
                    };
                }))
            }
            hir::Expression::ConditionNotify { condition, all } => {
                let condition = self.expression(condition)?;
                if *all {
                    Ok(quote!((#condition).notify_all()))
                } else {
                    Ok(quote!((#condition).notify_one()))
                }
            }
            hir::Expression::ThreadSpawn(callback) => {
                let callback = self.expression(callback)?;
                Ok(quote!(::std::thread::spawn(#callback)))
            }
            hir::Expression::ThreadJoin(handle) => {
                let handle = self.expression(handle)?;
                let panic = self.temporary("thread_panic")?;
                let message = self.temporary("thread_panic_message")?;
                Ok(quote!(match (#handle).join() {
                    Ok(value) => Ok(value),
                    Err(#panic) => {
                        let #message = if let Some(text) =
                            (#panic).downcast_ref::<::std::string::String>()
                        {
                            ::core::clone::Clone::clone(text)
                        } else if let Some(text) =
                            (#panic).downcast_ref::<&'static str>()
                        {
                            ::std::string::String::from(*text)
                        } else {
                            ::std::string::String::from(
                                "spawned Rust thread panicked with a non-string payload"
                            )
                        };
                        Err(Box::new(
                            crate::__stainless_namespace_stainless::ThreadError {
                                __stainless_base_Exception:
                                    crate::__stainless_namespace_stainless::Exception {
                                        message: #message,
                                    },
                            },
                        ) as crate::__StainlessExceptionBox)
                    }
                }))
            }
            hir::Expression::ThreadScope(callback) => {
                let callback = self.expression(callback)?;
                let panic = self.temporary("scoped_thread_panic")?;
                let message = self.temporary("scoped_thread_panic_message")?;
                Ok(quote!(match ::std::panic::catch_unwind(
                    ::std::panic::AssertUnwindSafe(|| ::std::thread::scope(#callback))
                ) {
                    Ok(value) => Ok(value),
                    Err(#panic) => {
                        let #message = if let Some(text) =
                            (#panic).downcast_ref::<::std::string::String>()
                        {
                            ::core::clone::Clone::clone(text)
                        } else if let Some(text) =
                            (#panic).downcast_ref::<&'static str>()
                        {
                            ::std::string::String::from(*text)
                        } else {
                            ::std::string::String::from(
                                "scoped Rust thread panicked with a non-string payload"
                            )
                        };
                        Err(Box::new(
                            crate::__stainless_namespace_stainless::ThreadError {
                                __stainless_base_Exception:
                                    crate::__stainless_namespace_stainless::Exception {
                                        message: #message,
                                    },
                            },
                        ) as crate::__StainlessExceptionBox)
                    }
                }))
            }
            hir::Expression::ScopedThreadSpawn { scope, callback } => {
                let scope = self.expression(scope)?;
                let callback = self.expression(callback)?;
                Ok(quote!((#scope).spawn(#callback)))
            }
            hir::Expression::ScopedThreadJoin(handle) => {
                let handle = self.expression(handle)?;
                let panic = self.temporary("joined_scoped_thread_panic")?;
                Ok(quote!(match (#handle).join() {
                    Ok(value) => value,
                    Err(#panic) => ::std::panic::resume_unwind(#panic),
                }))
            }
            hir::Expression::UnwrapRustResult {
                expression,
                exception,
                error_message,
                target,
            } => self.unwrap_rust_result(expression, *exception, *error_message, target),
            hir::Expression::Success(value) => {
                let value = value
                    .as_deref()
                    .map(|value| self.expression(value))
                    .transpose()?
                    .unwrap_or_else(|| quote!(()));
                Ok(quote!(Ok(#value)))
            }
            hir::Expression::Propagate { expression, target } => {
                let expression = self.expression(expression)?;
                let error = self.temporary("propagated_error")?;
                let propagation_error = quote!(#error);
                let propagation = exception_propagation(target, &propagation_error);
                Ok(quote! {
                    match #expression {
                        Ok(value) => value,
                        Err(#error) => {
                            #propagation
                        }
                    }
                })
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
            hir::Expression::Field {
                receiver,
                access_path,
            } => {
                let mut receiver = self.expression(receiver)?;
                for field in access_path {
                    if let Ok(index) = field.parse::<usize>() {
                        let index = syn::Index::from(index);
                        receiver = quote!((#receiver).#index);
                    } else {
                        let field = identifier(field)?;
                        receiver = quote!((#receiver).#field);
                    }
                }
                Ok(receiver)
            }
            hir::Expression::Aggregate { ty, fields } => {
                let ty = match ty {
                    hir::Type::User {
                        rust_path,
                        arguments,
                    } if !arguments.is_empty() => {
                        let path = path(rust_path)?;
                        let arguments = arguments
                            .iter()
                            .map(|argument| type_tokens(argument, None))
                            .collect::<Result<Vec<_>, _>>()?;
                        quote!(#path::<#(#arguments),*>)
                    }
                    _ => type_tokens(ty, None)?,
                };
                let fields = fields
                    .iter()
                    .map(|(name, value)| {
                        let name = identifier(name)?;
                        let value = self.expression(value)?;
                        Ok(quote!(#name: #value))
                    })
                    .collect::<Result<Vec<_>, String>>()?;
                Ok(quote!(#ty { #(#fields),* }))
            }
            hir::Expression::Lambda {
                captures,
                is_async,
                repeatable,
                parameters,
                body,
            } => {
                let capture_initializers = captures
                    .iter()
                    .map(|capture| {
                        let name = identifier(&capture.rust_name)?;
                        let mutable = capture.mutable.then(|| quote!(mut));
                        let initializer = self.expression(&capture.initializer)?;
                        Ok(quote!(let #mutable #name = #initializer;))
                    })
                    .collect::<Result<Vec<_>, String>>()?;
                let parameters = parameters
                    .iter()
                    .map(|parameter| parameter_tokens(parameter, None))
                    .collect::<Result<Vec<_>, _>>()?;
                let body = self.block(body)?;
                let async_capture_copies = (*is_async && *repeatable)
                    .then(|| {
                        captures
                            .iter()
                            .map(|capture| {
                                let name = identifier(&capture.rust_name)?;
                                Ok(quote!(
                                    let #name = ::core::clone::Clone::clone(&#name);
                                ))
                            })
                            .collect::<Result<Vec<_>, String>>()
                    })
                    .transpose()?
                    .unwrap_or_default();
                let closure = if *is_async {
                    quote!(move |#(#parameters),*| {
                        #(#async_capture_copies)*
                        async move #body
                    })
                } else {
                    quote!(move |#(#parameters),*| #body)
                };
                Ok(quote!({
                    #(#capture_initializers)*
                    #closure
                }))
            }
            hir::Expression::Await(expression) => {
                let expression = self.expression(expression)?;
                Ok(quote!((#expression).await))
            }
            hir::Expression::FunctionItem { modules, function } => {
                let target = if modules.is_empty() {
                    format!("crate::{function}")
                } else {
                    format!("crate::{}::{function}", modules.join("::"))
                };
                let target = path(&target)?;
                Ok(quote!(#target))
            }
            hir::Expression::StoreFunction { kind, ty, callable } => {
                let callable = self.expression(callable)?;
                let ty = type_tokens(ty, None)?;
                let allocation = match kind {
                    crate::interop::StoredFunctionKind::Shared => {
                        quote!(::std::sync::Arc::new(#callable))
                    }
                    crate::interop::StoredFunctionKind::Mutable => {
                        quote!(::std::boxed::Box::new(#callable))
                    }
                };
                Ok(quote!((#allocation as #ty)))
            }
            hir::Expression::FunctionCall { .. }
            | hir::Expression::InterfaceCall { .. }
            | hir::Expression::CallableCall { .. }
            | hir::Expression::AssociatedCall { .. }
            | hir::Expression::WrapperCall { .. }
            | hir::Expression::MethodCall { .. }
            | hir::Expression::Clone { .. }
            | hir::Expression::Cast { .. } => self.call_expression(expression),
        }
    }

    fn unwrap_rust_result(
        &mut self,
        expression: &hir::Expression,
        exception: hir::NativeExceptionKind,
        error_message: hir::RustErrorMessage,
        target: &hir::ExceptionTarget,
    ) -> Result<TokenStream, String> {
        let expression = self.expression(expression)?;
        let error = self.temporary("native_error")?;
        let message = self.temporary("native_error_message")?;
        let message_value = match error_message {
            hir::RustErrorMessage::Display => {
                quote!(::std::string::ToString::to_string(&#error))
            }
            hir::RustErrorMessage::Debug => quote!(::std::format!("{:?}", &#error)),
            hir::RustErrorMessage::Fallback => quote!({
                ::std::mem::drop(#error);
                ::std::string::String::from("native Rust operation failed")
            }),
        };
        let exception = match exception {
            hir::NativeExceptionKind::RustError => quote!(RustError),
            hir::NativeExceptionKind::IoError => quote!(IoError),
            hir::NativeExceptionKind::FormatError => quote!(FormatError),
            hir::NativeExceptionKind::JsonError => quote!(JsonError),
        };
        let converted = quote! {
            Box::new(crate::__stainless_namespace_stainless::#exception {
                __stainless_base_Exception: crate::__stainless_namespace_stainless::Exception {
                    message: #message,
                },
            }) as crate::__StainlessExceptionBox
        };
        let propagation = exception_propagation(target, &converted);
        Ok(quote! {
            match #expression {
                Ok(value) => value,
                Err(#error) => {
                    let #message: ::std::string::String = #message_value;
                    #propagation
                }
            }
        })
    }

    fn call_expression(&mut self, expression: &hir::Expression) -> Result<TokenStream, String> {
        match expression {
            hir::Expression::CallableCall {
                callable,
                arguments,
            } => {
                let callable = self.expression(callable)?;
                let arguments = arguments
                    .iter()
                    .map(|argument| self.expression(argument))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(quote!((#callable)(#(#arguments),*)))
            }
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
            hir::Expression::InterfaceCall {
                receiver,
                method,
                arguments,
            } => {
                let receiver = self.expression(receiver)?;
                let method = identifier(method)?;
                let arguments = arguments
                    .iter()
                    .map(|argument| self.expression(argument))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(quote!((#receiver).#method(#(#arguments),*)))
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
            hir::Expression::WrapperCall {
                rust_name,
                arguments,
            } => {
                let rust_name = identifier(rust_name)?;
                let arguments = arguments
                    .iter()
                    .map(|argument| self.expression(argument))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(quote!(crate::__stainless_bindings::#rust_name(
                    #(#arguments),*
                )))
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
                Ok(quote!(((#expression) as #target)))
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

fn generic_parameter_declarations(
    parameters: &[String],
    const_parameters: &[String],
    static_type_bound: bool,
) -> Result<Vec<TokenStream>, String> {
    parameters
        .iter()
        .map(|parameter| {
            let name = identifier(parameter)?;
            if const_parameters.contains(parameter) {
                Ok(quote!(const #name: usize))
            } else if static_type_bound {
                Ok(quote!(#name: 'static))
            } else {
                Ok(quote!(#name))
            }
        })
        .collect()
}

#[allow(clippy::too_many_lines)]
fn type_tokens(ty: &hir::Type, lifetime: Option<&syn::Lifetime>) -> Result<TokenStream, String> {
    match ty {
        hir::Type::Unit => Ok(quote!(())),
        hir::Type::Primitive(name) => {
            let name = identifier(name)?;
            Ok(quote!(#name))
        }
        hir::Type::ConstUsize(value) => {
            let value = syn::LitInt::new(&value.to_string(), proc_macro2::Span::call_site());
            Ok(quote!(#value))
        }
        hir::Type::ConstParameter(name) | hir::Type::Parameter(name) => {
            let name = identifier(name)?;
            Ok(quote!(#name))
        }
        hir::Type::Array { element, length } => {
            let element = type_tokens(element, None)?;
            let length = type_tokens(length, None)?;
            Ok(quote!([#element; #length]))
        }
        hir::Type::Tuple(elements) => {
            let elements = elements
                .iter()
                .map(|element| type_tokens(element, None))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(quote!((#(#elements),*)))
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
        hir::Type::ClassBase(target) => {
            let target = type_tokens(target, None)?;
            Ok(quote!(::stainless_runtime::ClassBase<#target>))
        }
        hir::Type::Callback { .. } => {
            Err("callback type escaped its generated-wrapper parameter boundary".to_owned())
        }
        hir::Type::Function {
            kind,
            parameters,
            return_type,
        } => {
            let parameters = parameters
                .iter()
                .map(|parameter| type_tokens(parameter, None))
                .collect::<Result<Vec<_>, _>>()?;
            let return_type = type_tokens(return_type, None)?;
            Ok(match kind {
                crate::interop::StoredFunctionKind::Shared => quote!(
                    ::std::sync::Arc<
                        dyn ::core::ops::Fn(#(#parameters),*) -> #return_type + 'static
                    >
                ),
                crate::interop::StoredFunctionKind::Mutable => quote!(
                    ::std::boxed::Box<
                        dyn ::core::ops::FnMut(#(#parameters),*) -> #return_type + 'static
                    >
                ),
            })
        }
        hir::Type::Pointer { kind, target } => {
            let target = type_tokens(target, None)?;
            Ok(match kind {
                crate::interop::PointerKind::Unique => quote!(::std::boxed::Box<#target>),
                crate::interop::PointerKind::UniqueNullable => {
                    quote!(::core::option::Option<::std::boxed::Box<#target>>)
                }
                crate::interop::PointerKind::Shared => quote!(::std::sync::Arc<#target>),
                crate::interop::PointerKind::SharedNullable => {
                    quote!(::core::option::Option<::std::sync::Arc<#target>>)
                }
                crate::interop::PointerKind::Weak => quote!(::std::sync::Weak<#target>),
                crate::interop::PointerKind::Atomic => {
                    quote!(::std::sync::RwLock<::std::sync::Arc<#target>>)
                }
                crate::interop::PointerKind::AtomicNullable => quote!(
                    ::std::sync::RwLock<::core::option::Option<::std::sync::Arc<#target>>>
                ),
            })
        }
        hir::Type::Mutex(target) => {
            let target = type_tokens(target, None)?;
            Ok(quote!(::std::sync::Mutex<#target>))
        }
        hir::Type::MutexGuard(target) => {
            let target = type_tokens(target, None)?;
            Ok(quote!(::std::sync::MutexGuard<'_, #target>))
        }
        hir::Type::RwLock(target) => {
            let target = type_tokens(target, None)?;
            Ok(quote!(::std::sync::RwLock<#target>))
        }
        hir::Type::RwLockReadGuard(target) => {
            let target = type_tokens(target, None)?;
            Ok(quote!(::std::sync::RwLockReadGuard<'_, #target>))
        }
        hir::Type::RwLockWriteGuard(target) => {
            let target = type_tokens(target, None)?;
            Ok(quote!(::std::sync::RwLockWriteGuard<'_, #target>))
        }
        hir::Type::Condition => Ok(quote!(::std::sync::Condvar)),
        hir::Type::ThreadHandle(target) => {
            let target = type_tokens(target, None)?;
            Ok(quote!(::std::thread::JoinHandle<#target>))
        }
        hir::Type::ThreadScope => Ok(quote!(::std::thread::Scope<'_, '_>)),
        hir::Type::ScopedThreadHandle(target) => {
            let target = type_tokens(target, None)?;
            Ok(quote!(::std::thread::ScopedJoinHandle<'_, #target>))
        }
        hir::Type::User {
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
                Ok(quote!(#path<#(#arguments),*>))
            }
        }
        hir::Type::Interface {
            rust_path,
            arguments,
        } => {
            let path = path(rust_path)?;
            let arguments = arguments
                .iter()
                .map(|argument| type_tokens(argument, None))
                .collect::<Result<Vec<_>, _>>()?;
            if arguments.is_empty() {
                Ok(quote!(dyn #path + ::core::marker::Send + ::core::marker::Sync))
            } else {
                Ok(quote!(dyn #path<#(#arguments),*> + ::core::marker::Send + ::core::marker::Sync))
            }
        }
        hir::Type::Reference { mutable, target } => {
            let interface = matches!(target.as_ref(), hir::Type::Interface { .. });
            let target = type_tokens(target, None)?;
            let mutable = mutable.then(|| quote!(mut));
            if interface {
                Ok(quote!(& #lifetime #mutable (#target)))
            } else {
                Ok(quote!(& #lifetime #mutable #target))
            }
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
        LiteralKind::Null => Err("JSON null reached scalar literal emission".to_owned()),
    }
}

fn switch_literal(kind: LiteralKind, text: &str) -> Result<TokenStream, String> {
    if kind == LiteralKind::String {
        let literal = syn::parse_str::<syn::LitStr>(text)
            .map_err(|error| format!("invalid string literal `{text}`: {error}"))?;
        Ok(quote!(#literal))
    } else {
        literal(kind, text)
    }
}

fn checked_index(index: &TokenStream) -> TokenStream {
    quote!(
        <usize as ::core::convert::TryFrom<_>>::try_from(#index)
            .expect("Stainless index does not fit usize")
    )
}

fn json_cast_method(target: &hir::Type) -> Result<&'static str, String> {
    match target {
        hir::Type::Primitive("bool") => Ok("to_bool"),
        hir::Type::Primitive("i8") => Ok("to_i8"),
        hir::Type::Primitive("i16") => Ok("to_i16"),
        hir::Type::Primitive("i32") => Ok("to_i32"),
        hir::Type::Primitive("i64") => Ok("to_i64"),
        hir::Type::Primitive("i128") => Ok("to_i128"),
        hir::Type::Primitive("isize") => Ok("to_isize"),
        hir::Type::Primitive("u8") => Ok("to_u8"),
        hir::Type::Primitive("u16") => Ok("to_u16"),
        hir::Type::Primitive("u32") => Ok("to_u32"),
        hir::Type::Primitive("u64") => Ok("to_u64"),
        hir::Type::Primitive("u128") => Ok("to_u128"),
        hir::Type::Primitive("usize") => Ok("to_usize"),
        hir::Type::Primitive("f32") => Ok("to_f32"),
        hir::Type::Primitive("f64") => Ok("to_f64"),
        hir::Type::Native {
            rust_path,
            arguments,
        } if rust_path == "::std::string::String" && arguments.is_empty() => Ok("to_string_value"),
        _ => Err("unsupported JSON scalar conversion reached code generation".to_owned()),
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

fn exception_propagation(target: &hir::ExceptionTarget, error: &TokenStream) -> TokenStream {
    match target {
        hir::ExceptionTarget::Function => quote!(return Err(#error);),
        hir::ExceptionTarget::Try(label) => {
            let label = syn::Lifetime::new(&format!("'{label}"), TokenSpan::call_site());
            quote!(break #label Err(#error);)
        }
        hir::ExceptionTarget::Unreachable => quote! {
            unreachable!("statically exhaustive Stainless catch set missed an exception");
        },
    }
}

fn rust_label(label: &str) -> syn::Lifetime {
    syn::Lifetime::new(&format!("'{label}"), TokenSpan::call_site())
}

fn program_has_exceptions(program: &hir::Program) -> bool {
    fn module_has_exceptions(module: &hir::Module) -> bool {
        module
            .structs
            .iter()
            .any(|structure| structure.is_exception)
            || module.modules.iter().any(module_has_exceptions)
    }

    program
        .structs
        .iter()
        .any(|structure| structure.is_exception)
        || program.modules.iter().any(module_has_exceptions)
}
