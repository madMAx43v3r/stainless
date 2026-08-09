use super::model::{
    ArgumentAdaptation, CallStyle, CallableBinding, CallbackEscape, CallbackKind, NativeBindings,
    NativeErrorFormat, NativeTypeBinding, Parameter, Receiver, RustLowering, TraitRequirement,
    TypeRef,
};

const T: &str = "T";
const K: &str = "K";
const V: &str = "V";

/// Returns the compiler-provided native Rust bindings implemented so far.
///
/// These bindings expose real Rust standard-library representations.
/// They are semantic/code-generation metadata, not runtime wrapper newtypes.
///
/// # Errors
///
/// Returns an error if compiler-provided metadata violates registry
/// invariants. Such an error indicates a compiler implementation defect.
pub fn standard_bindings() -> Result<NativeBindings, super::BindingError> {
    NativeBindings::new(vec![
        big_endian_binding(),
        clock_binding(),
        little_endian_binding(),
        fs_binding(),
        file_binding(),
        open_options_binding(),
        io_error_binding(),
        json_error_binding(),
        list_binding(),
        map_binding(),
        multimap_binding(),
        option_binding(),
        queue_binding(),
        random_binding(),
        random_error_binding(),
        set_binding(),
        string_binding(),
        string_from_utf8_error_binding(),
        var_binding(),
        vec_binding(),
    ])
}

fn big_endian_binding() -> NativeTypeBinding {
    endian_binding("BigEndian")
}

fn little_endian_binding() -> NativeTypeBinding {
    endian_binding("LittleEndian")
}

#[allow(clippy::too_many_lines)]
fn endian_binding(name: &str) -> NativeTypeBinding {
    let bytes = vec_of(TypeRef::U8);
    let io_error = TypeRef::native(IO_ERROR_TYPE_PATH, Vec::new());
    let write_parameters = |value| {
        vec![
            Parameter::new("output", TypeRef::mutable_ref(bytes.clone())),
            Parameter::new("value", value),
        ]
    };
    let read_parameters = || vec![Parameter::new("bytes", TypeRef::shared_ref(bytes.clone()))];
    let read_at_parameters = || {
        vec![
            Parameter::new("bytes", TypeRef::shared_ref(bytes.clone())),
            Parameter::new("offset", TypeRef::Usize),
        ]
    };
    let read_advance_parameters = |value| {
        vec![
            Parameter::new("bytes", TypeRef::shared_ref(bytes.clone())),
            Parameter::new("offset", TypeRef::mutable_ref(TypeRef::Usize)),
            Parameter::new("value", TypeRef::mutable_ref(value)),
        ]
    };
    let rust_path = format!("::stainless_runtime::{name}");
    let mut callables = vec![
        associated(
            "write_u32",
            write_parameters(TypeRef::U32),
            TypeRef::Void,
            &format!("{rust_path}::write_u32"),
        ),
        associated(
            "write_u64",
            write_parameters(TypeRef::U64),
            TypeRef::Void,
            &format!("{rust_path}::write_u64"),
        ),
        fallible_associated(
            "write_u32",
            write_parameters(TypeRef::Usize),
            TypeRef::Void,
            io_error.clone(),
            &format!("{rust_path}::write_usize_u32"),
        ),
        associated(
            "write_u64",
            write_parameters(TypeRef::Usize),
            TypeRef::Void,
            &format!("{rust_path}::write_usize_u64"),
        ),
        fallible_associated(
            "read_u8",
            read_parameters(),
            TypeRef::U8,
            io_error.clone(),
            &format!("{rust_path}::read_u8"),
        ),
        fallible_associated(
            "read_u8_at",
            read_at_parameters(),
            TypeRef::U8,
            io_error.clone(),
            &format!("{rust_path}::read_u8_at"),
        ),
        fallible_associated(
            "read_u32",
            read_parameters(),
            TypeRef::U32,
            io_error.clone(),
            &format!("{rust_path}::read_u32"),
        ),
        fallible_associated(
            "read_u32_at",
            read_at_parameters(),
            TypeRef::U32,
            io_error.clone(),
            &format!("{rust_path}::read_u32_at"),
        ),
        fallible_associated(
            "read_u64",
            read_parameters(),
            TypeRef::U64,
            io_error.clone(),
            &format!("{rust_path}::read_u64"),
        ),
        fallible_associated(
            "read_u64_at",
            read_at_parameters(),
            TypeRef::U64,
            io_error.clone(),
            &format!("{rust_path}::read_u64_at"),
        ),
    ];
    if name == "BigEndian" {
        for (source_name, ty, rust_name) in [
            ("write_u8", TypeRef::U8, "write_u8"),
            ("write_u16", TypeRef::U16, "write_u16"),
            ("write_u128", TypeRef::U128, "write_u128"),
            ("write", TypeRef::U8, "write_u8"),
            ("write", TypeRef::U16, "write_u16"),
            ("write", TypeRef::U32, "write_u32"),
            ("write", TypeRef::U64, "write_u64"),
            ("write", TypeRef::U128, "write_u128"),
        ] {
            callables.push(associated(
                source_name,
                write_parameters(ty),
                TypeRef::Void,
                &format!("{rust_path}::{rust_name}"),
            ));
        }
        for (source_name, ty, rust_name) in [
            ("read_u16", TypeRef::U16, "read_u16"),
            ("read_u128", TypeRef::U128, "read_u128"),
        ] {
            callables.push(fallible_associated(
                source_name,
                read_parameters(),
                ty,
                io_error.clone(),
                &format!("{rust_path}::{rust_name}"),
            ));
        }
        for (source_name, ty, rust_name) in [
            ("read_u16_at", TypeRef::U16, "read_u16_at"),
            ("read_u128_at", TypeRef::U128, "read_u128_at"),
        ] {
            callables.push(fallible_associated(
                source_name,
                read_at_parameters(),
                ty,
                io_error.clone(),
                &format!("{rust_path}::{rust_name}"),
            ));
        }
        for (ty, rust_name) in [
            (TypeRef::U8, "read_u8_advance"),
            (TypeRef::U16, "read_u16_advance"),
            (TypeRef::U32, "read_u32_advance"),
            (TypeRef::U64, "read_u64_advance"),
            (TypeRef::U128, "read_u128_advance"),
        ] {
            callables.push(fallible_associated(
                "read",
                read_advance_parameters(ty),
                TypeRef::Void,
                io_error.clone(),
                &format!("{rust_path}::{rust_name}"),
            ));
        }
    }

    NativeTypeBinding {
        stainless_path: format!("rust::stainless_runtime::{name}"),
        rust_path: rust_path.clone(),
        type_parameters: vec![],
        error_format: None,
        callables,
    }
}

pub(crate) const VAR_TYPE_PATH: &str = "rust::stainless_runtime::Var";
const JSON_ERROR_TYPE_PATH: &str = "rust::stainless_runtime::JsonError";
const RANDOM_ERROR_TYPE_PATH: &str = "rust::stainless_runtime::RandomError";
const IO_ERROR_TYPE_PATH: &str = "rust::std::io::Error";
const FILE_TYPE_PATH: &str = "rust::std::fs::File";
const OPEN_OPTIONS_TYPE_PATH: &str = "rust::std::fs::OpenOptions";

fn fs_binding() -> NativeTypeBinding {
    let string = string_type();
    let string_ref = TypeRef::shared_ref(string.clone());
    let bytes = vec_of(TypeRef::U8);
    let bytes_ref = TypeRef::shared_ref(bytes.clone());
    let io_error = TypeRef::native(IO_ERROR_TYPE_PATH, Vec::new());
    let mut callables = fs_file_callables(&string, &string_ref, &bytes, &bytes_ref, &io_error);
    callables.extend(fs_directory_callables(&string_ref, &io_error));

    NativeTypeBinding {
        stainless_path: "rust::std::fs".to_owned(),
        rust_path: "::stainless_runtime::Fs".to_owned(),
        type_parameters: vec![],
        error_format: None,
        callables,
    }
}

#[allow(clippy::too_many_lines)]
fn file_binding() -> NativeTypeBinding {
    let file = TypeRef::native(FILE_TYPE_PATH, Vec::new());
    let string_ref = TypeRef::shared_ref(string_type());
    let bytes = vec_of(TypeRef::U8);
    let io_error = TypeRef::native(IO_ERROR_TYPE_PATH, Vec::new());

    NativeTypeBinding {
        stainless_path: FILE_TYPE_PATH.to_owned(),
        rust_path: "::stainless_runtime::PositionedFile".to_owned(),
        type_parameters: vec![],
        error_format: None,
        callables: vec![
            constructor(
                "File",
                vec![Parameter::new(
                    "file",
                    TypeRef::native(FILE_TYPE_PATH, Vec::new()),
                )],
                TypeRef::native(FILE_TYPE_PATH, Vec::new()),
                RustLowering::AssociatedFunction {
                    rust_path: "::stainless_runtime::PositionedFile::from_owned".to_owned(),
                },
            ),
            fallible_associated(
                "open",
                vec![fs_path_parameter("path", &string_ref)],
                file,
                io_error.clone(),
                "::stainless_runtime::PositionedFile::open",
            ),
            fallible_associated(
                "create",
                vec![fs_path_parameter("path", &string_ref)],
                TypeRef::native(FILE_TYPE_PATH, Vec::new()),
                io_error.clone(),
                "::stainless_runtime::PositionedFile::create",
            ),
            fallible_method(
                "pread",
                "pread",
                Receiver::Shared,
                vec![
                    Parameter::new("offset", TypeRef::U64),
                    Parameter::new("length", TypeRef::Usize),
                ],
                bytes.clone(),
                io_error,
            ),
            fallible_method(
                "pread_exact",
                "pread_exact",
                Receiver::Shared,
                vec![
                    Parameter::new("offset", TypeRef::U64),
                    Parameter::new("length", TypeRef::Usize),
                ],
                bytes.clone(),
                TypeRef::native(IO_ERROR_TYPE_PATH, Vec::new()),
            ),
            fallible_method(
                "pwrite",
                "pwrite",
                Receiver::Shared,
                vec![
                    Parameter::new("offset", TypeRef::U64),
                    Parameter::new("contents", TypeRef::shared_ref(bytes.clone())),
                ],
                TypeRef::Usize,
                TypeRef::native(IO_ERROR_TYPE_PATH, Vec::new()),
            ),
            fallible_method(
                "pwrite_all",
                "pwrite_all",
                Receiver::Shared,
                vec![
                    Parameter::new("offset", TypeRef::U64),
                    Parameter::new("contents", TypeRef::shared_ref(bytes.clone())),
                ],
                TypeRef::Void,
                TypeRef::native(IO_ERROR_TYPE_PATH, Vec::new()),
            ),
            fallible_method(
                "sync_all",
                "sync_all",
                Receiver::Shared,
                vec![],
                TypeRef::Void,
                TypeRef::native(IO_ERROR_TYPE_PATH, Vec::new()),
            ),
            fallible_method(
                "sync_data",
                "sync_data",
                Receiver::Shared,
                vec![],
                TypeRef::Void,
                TypeRef::native(IO_ERROR_TYPE_PATH, Vec::new()),
            ),
            fallible_method(
                "set_len",
                "set_len",
                Receiver::Shared,
                vec![Parameter::new("size", TypeRef::U64)],
                TypeRef::Void,
                TypeRef::native(IO_ERROR_TYPE_PATH, Vec::new()),
            ),
            fallible_method(
                "len",
                "len",
                Receiver::Shared,
                vec![],
                TypeRef::U64,
                TypeRef::native(IO_ERROR_TYPE_PATH, Vec::new()),
            ),
            fallible_method(
                "is_empty",
                "is_empty",
                Receiver::Shared,
                vec![],
                TypeRef::Bool,
                TypeRef::native(IO_ERROR_TYPE_PATH, Vec::new()),
            ),
            fallible_method(
                "try_clone",
                "try_clone",
                Receiver::Shared,
                vec![],
                TypeRef::native(FILE_TYPE_PATH, Vec::new()),
                TypeRef::native(IO_ERROR_TYPE_PATH, Vec::new()),
            ),
        ],
    }
}

fn open_options_binding() -> NativeTypeBinding {
    let options = TypeRef::native(OPEN_OPTIONS_TYPE_PATH, Vec::new());
    let string_ref = TypeRef::shared_ref(string_type());
    let file = TypeRef::native(FILE_TYPE_PATH, Vec::new());
    let io_error = TypeRef::native(IO_ERROR_TYPE_PATH, Vec::new());
    let mut callables = vec![constructor(
        "OpenOptions",
        vec![],
        options,
        RustLowering::AssociatedFunction {
            rust_path: "::stainless_runtime::PositionedOpenOptions::new".to_owned(),
        },
    )];
    for name in [
        "read",
        "write",
        "append",
        "truncate",
        "create",
        "create_new",
    ] {
        callables.push(method(
            name,
            Receiver::Mutable,
            vec![Parameter::new("enabled", TypeRef::Bool)],
            TypeRef::Void,
        ));
    }
    callables.push(fallible_method(
        "open",
        "open",
        Receiver::Shared,
        vec![fs_path_parameter("path", &string_ref)],
        file,
        io_error,
    ));

    NativeTypeBinding {
        stainless_path: OPEN_OPTIONS_TYPE_PATH.to_owned(),
        rust_path: "::stainless_runtime::PositionedOpenOptions".to_owned(),
        type_parameters: vec![],
        error_format: None,
        callables,
    }
}

fn fs_file_callables(
    string: &TypeRef,
    string_ref: &TypeRef,
    bytes: &TypeRef,
    bytes_ref: &TypeRef,
    io_error: &TypeRef,
) -> Vec<CallableBinding> {
    vec![
        fallible_associated(
            "read_to_string",
            vec![fs_path_parameter("path", string_ref)],
            string.clone(),
            io_error.clone(),
            "::stainless_runtime::Fs::read_to_string",
        ),
        fallible_associated(
            "read",
            vec![fs_path_parameter("path", string_ref)],
            bytes.clone(),
            io_error.clone(),
            "::stainless_runtime::Fs::read",
        ),
        fallible_associated(
            "write",
            vec![
                fs_path_parameter("path", string_ref),
                Parameter::adapted(
                    "contents",
                    string_ref.clone(),
                    ArgumentAdaptation::StringRefToStr,
                ),
            ],
            TypeRef::Void,
            io_error.clone(),
            "::stainless_runtime::Fs::write_text",
        ),
        fallible_associated(
            "write",
            vec![
                fs_path_parameter("path", string_ref),
                Parameter::new("contents", bytes_ref.clone()),
            ],
            TypeRef::Void,
            io_error.clone(),
            "::stainless_runtime::Fs::write_bytes",
        ),
        fallible_associated(
            "exists",
            vec![fs_path_parameter("path", string_ref)],
            TypeRef::Bool,
            io_error.clone(),
            "::stainless_runtime::Fs::exists",
        ),
        fallible_associated(
            "copy",
            vec![
                fs_path_parameter("from", string_ref),
                fs_path_parameter("to", string_ref),
            ],
            TypeRef::U64,
            io_error.clone(),
            "::stainless_runtime::Fs::copy",
        ),
        fallible_associated(
            "rename",
            vec![
                fs_path_parameter("from", string_ref),
                fs_path_parameter("to", string_ref),
            ],
            TypeRef::Void,
            io_error.clone(),
            "::stainless_runtime::Fs::rename",
        ),
        fallible_associated(
            "remove_file",
            vec![fs_path_parameter("path", string_ref)],
            TypeRef::Void,
            io_error.clone(),
            "::stainless_runtime::Fs::remove_file",
        ),
    ]
}

fn fs_directory_callables(string_ref: &TypeRef, io_error: &TypeRef) -> Vec<CallableBinding> {
    [
        "create_dir",
        "create_dir_all",
        "remove_dir",
        "remove_dir_all",
    ]
    .into_iter()
    .map(|name| {
        fallible_associated(
            name,
            vec![fs_path_parameter("path", string_ref)],
            TypeRef::Void,
            io_error.clone(),
            match name {
                "create_dir" => "::stainless_runtime::Fs::create_dir",
                "create_dir_all" => "::stainless_runtime::Fs::create_dir_all",
                "remove_dir" => "::stainless_runtime::Fs::remove_dir",
                "remove_dir_all" => "::stainless_runtime::Fs::remove_dir_all",
                _ => unreachable!("filesystem directory binding name is fixed"),
            },
        )
    })
    .collect()
}

fn fs_path_parameter(name: &'static str, string_ref: &TypeRef) -> Parameter {
    Parameter::adapted(name, string_ref.clone(), ArgumentAdaptation::StringRefToStr)
}

fn io_error_binding() -> NativeTypeBinding {
    NativeTypeBinding {
        stainless_path: IO_ERROR_TYPE_PATH.to_owned(),
        rust_path: "::std::io::Error".to_owned(),
        type_parameters: vec![],
        error_format: Some(NativeErrorFormat::Display),
        callables: vec![],
    }
}

#[allow(clippy::too_many_lines)]
fn var_binding() -> NativeTypeBinding {
    let var = TypeRef::native(VAR_TYPE_PATH, Vec::new());
    let string = string_type();
    let string_ref = TypeRef::shared_ref(string.clone());
    let bytes_ref = TypeRef::shared_ref(vec_of(TypeRef::U8));
    let json_error = TypeRef::native(JSON_ERROR_TYPE_PATH, Vec::new());
    let result_var = TypeRef::native("rust::Result", vec![var.clone(), json_error.clone()]);

    NativeTypeBinding {
        stainless_path: VAR_TYPE_PATH.to_owned(),
        rust_path: "::stainless_runtime::Var".to_owned(),
        type_parameters: vec![],
        error_format: None,
        callables: vec![
            constructor(
                "Var",
                vec![],
                var.clone(),
                RustLowering::AssociatedFunction {
                    rust_path: "::stainless_runtime::Var::null".to_owned(),
                },
            ),
            CallableBinding {
                source_name: "parse".to_owned(),
                style: CallStyle::AssociatedFunction,
                receiver: None,
                parameters: vec![Parameter::adapted(
                    "source",
                    string_ref.clone(),
                    ArgumentAdaptation::StringRefToStr,
                )],
                is_async: false,
                return_type: result_var.clone(),
                rust_result_error: None,
                return_borrow: None,
                requirements: vec![],
                lowering: RustLowering::AssociatedFunction {
                    rust_path: "::stainless_runtime::Var::parse".to_owned(),
                },
            },
            CallableBinding {
                source_name: "parse_bytes".to_owned(),
                style: CallStyle::AssociatedFunction,
                receiver: None,
                parameters: vec![Parameter::new("source", bytes_ref)],
                is_async: false,
                return_type: result_var.clone(),
                rust_result_error: None,
                return_borrow: None,
                requirements: vec![],
                lowering: RustLowering::AssociatedFunction {
                    rust_path: "::stainless_runtime::Var::parse_bytes".to_owned(),
                },
            },
            CallableBinding {
                source_name: "parse_file".to_owned(),
                style: CallStyle::AssociatedFunction,
                receiver: None,
                parameters: vec![Parameter::adapted(
                    "path",
                    string_ref.clone(),
                    ArgumentAdaptation::StringRefToStr,
                )],
                is_async: false,
                return_type: result_var,
                rust_result_error: None,
                return_borrow: None,
                requirements: vec![],
                lowering: RustLowering::AssociatedFunction {
                    rust_path: "::stainless_runtime::Var::parse_file".to_owned(),
                },
            },
            method("is_null", Receiver::Shared, vec![], TypeRef::Bool),
            method("is_string", Receiver::Shared, vec![], TypeRef::Bool),
            method("clone", Receiver::Shared, vec![], var),
            method("to_json", Receiver::Shared, vec![], string),
            renamed_method(
                "get",
                "field",
                Receiver::Shared,
                vec![Parameter::adapted(
                    "name",
                    string_ref.clone(),
                    ArgumentAdaptation::StringRefToStr,
                )],
                TypeRef::native(VAR_TYPE_PATH, Vec::new()),
            ),
            fallible_method(
                "set",
                "set_field",
                Receiver::Mutable,
                vec![
                    Parameter::adapted(
                        "name",
                        string_ref.clone(),
                        ArgumentAdaptation::StringRefToStr,
                    ),
                    Parameter::new("value", TypeRef::native(VAR_TYPE_PATH, Vec::new())),
                ],
                TypeRef::Void,
                json_error.clone(),
            ),
            fallible_method(
                "push",
                "push",
                Receiver::Mutable,
                vec![Parameter::new(
                    "value",
                    TypeRef::native(VAR_TYPE_PATH, Vec::new()),
                )],
                TypeRef::Void,
                json_error.clone(),
            ),
            fallible_method(
                "pop",
                "pop",
                Receiver::Mutable,
                vec![],
                TypeRef::native(VAR_TYPE_PATH, Vec::new()),
                json_error.clone(),
            ),
            fallible_method(
                "insert",
                "insert",
                Receiver::Mutable,
                vec![
                    Parameter::new("index", TypeRef::Usize),
                    Parameter::new("value", TypeRef::native(VAR_TYPE_PATH, Vec::new())),
                ],
                TypeRef::Void,
                json_error.clone(),
            ),
            fallible_method(
                "remove",
                "remove_index",
                Receiver::Mutable,
                vec![Parameter::new("index", TypeRef::Usize)],
                TypeRef::native(VAR_TYPE_PATH, Vec::new()),
                json_error.clone(),
            ),
            fallible_method(
                "remove",
                "remove_field",
                Receiver::Mutable,
                vec![Parameter::adapted(
                    "name",
                    string_ref.clone(),
                    ArgumentAdaptation::StringRefToStr,
                )],
                TypeRef::native(VAR_TYPE_PATH, Vec::new()),
                json_error.clone(),
            ),
            fallible_method(
                "clear",
                "clear",
                Receiver::Mutable,
                vec![],
                TypeRef::Void,
                json_error.clone(),
            ),
            fallible_method(
                "len",
                "len",
                Receiver::Shared,
                vec![],
                TypeRef::Usize,
                json_error.clone(),
            ),
            fallible_method(
                "is_empty",
                "is_empty",
                Receiver::Shared,
                vec![],
                TypeRef::Bool,
                json_error.clone(),
            ),
            fallible_method(
                "contains_key",
                "contains_key",
                Receiver::Shared,
                vec![Parameter::adapted(
                    "name",
                    string_ref,
                    ArgumentAdaptation::StringRefToStr,
                )],
                TypeRef::Bool,
                json_error,
            ),
        ],
    }
}

fn json_error_binding() -> NativeTypeBinding {
    NativeTypeBinding {
        stainless_path: JSON_ERROR_TYPE_PATH.to_owned(),
        rust_path: "::stainless_runtime::JsonError".to_owned(),
        type_parameters: vec![],
        error_format: Some(NativeErrorFormat::Display),
        callables: vec![],
    }
}

fn clock_binding() -> NativeTypeBinding {
    NativeTypeBinding {
        stainless_path: "rust::stainless_runtime::Clock".to_owned(),
        rust_path: "::stainless_runtime::Clock".to_owned(),
        type_parameters: vec![],
        error_format: None,
        callables: vec![associated(
            "unix_seconds",
            vec![],
            TypeRef::U64,
            "::stainless_runtime::Clock::unix_seconds",
        )],
    }
}

fn random_binding() -> NativeTypeBinding {
    NativeTypeBinding {
        stainless_path: "rust::stainless_runtime::Random".to_owned(),
        rust_path: "::stainless_runtime::Random".to_owned(),
        type_parameters: vec![],
        error_format: None,
        callables: vec![fallible_associated(
            "bytes",
            vec![Parameter::new("length", TypeRef::Usize)],
            vec_of(TypeRef::U8),
            TypeRef::native(RANDOM_ERROR_TYPE_PATH, vec![]),
            "::stainless_runtime::Random::bytes",
        )],
    }
}

fn random_error_binding() -> NativeTypeBinding {
    NativeTypeBinding {
        stainless_path: RANDOM_ERROR_TYPE_PATH.to_owned(),
        rust_path: "::stainless_runtime::RandomError".to_owned(),
        type_parameters: vec![],
        error_format: Some(NativeErrorFormat::Display),
        callables: vec![],
    }
}

fn vec_binding() -> NativeTypeBinding {
    let t = TypeRef::Parameter(T.to_owned());
    let vec_t = vec_of(t.clone());
    let range_visitor = TypeRef::callback(
        CallbackKind::FnMut,
        CallbackEscape::Call,
        vec![TypeRef::shared_ref(t.clone())],
        TypeRef::Void,
    );
    let mut callables = vec_construction(&vec_t);
    callables.extend(vec_capacity_methods());
    callables.extend(vec_element_methods(&t, &vec_t));
    callables.extend(vec_trait_methods(&t, &vec_t));
    callables.push(CallableBinding {
        source_name: "copy_range".to_owned(),
        style: CallStyle::Method,
        receiver: Some(Receiver::Shared),
        parameters: vec![
            Parameter::new("begin", TypeRef::Usize),
            Parameter::new("end", TypeRef::Usize),
        ],
        is_async: false,
        return_type: vec_t.clone(),
        rust_result_error: None,
        return_borrow: None,
        requirements: vec![requirement(T, "::core::clone::Clone")],
        lowering: RustLowering::FunctionWithReceiver {
            rust_path: "::stainless_runtime::vec_copy_range".to_owned(),
        },
    });
    callables.push(CallableBinding {
        source_name: "with_range".to_owned(),
        style: CallStyle::Method,
        receiver: Some(Receiver::Shared),
        parameters: vec![
            Parameter::new("begin", TypeRef::Usize),
            Parameter::new("end", TypeRef::Usize),
            Parameter::new("callback", range_visitor),
        ],
        is_async: false,
        return_type: TypeRef::Bool,
        rust_result_error: None,
        return_borrow: None,
        requirements: vec![],
        lowering: RustLowering::FunctionWithReceiver {
            rust_path: "::stainless_runtime::vec_with_range".to_owned(),
        },
    });

    NativeTypeBinding {
        stainless_path: "rust::Vec".to_owned(),
        rust_path: "::std::vec::Vec".to_owned(),
        type_parameters: vec![T.to_owned()],
        error_format: None,
        callables,
    }
}

fn list_binding() -> NativeTypeBinding {
    let t = TypeRef::Parameter(T.to_owned());
    let list_t = list_of(t.clone());

    NativeTypeBinding {
        stainless_path: "rust::List".to_owned(),
        rust_path: "::std::collections::LinkedList".to_owned(),
        type_parameters: vec![T.to_owned()],
        error_format: None,
        callables: vec![
            constructor(
                "List",
                vec![],
                list_t.clone(),
                RustLowering::AssociatedFunction {
                    rust_path: "::std::collections::LinkedList::new".to_owned(),
                },
            ),
            method("len", Receiver::Shared, vec![], TypeRef::Usize),
            method("is_empty", Receiver::Shared, vec![], TypeRef::Bool),
            method("clear", Receiver::Mutable, vec![], TypeRef::Void),
            method(
                "push_front",
                Receiver::Mutable,
                vec![Parameter::new("value", t.clone())],
                TypeRef::Void,
            ),
            method(
                "push_back",
                Receiver::Mutable,
                vec![Parameter::new("value", t.clone())],
                TypeRef::Void,
            ),
            method("pop_front", Receiver::Mutable, vec![], option_of(t.clone())),
            method("pop_back", Receiver::Mutable, vec![], option_of(t.clone())),
            method(
                "append",
                Receiver::Mutable,
                vec![Parameter::new(
                    "other",
                    TypeRef::mutable_ref(list_t.clone()),
                )],
                TypeRef::Void,
            ),
            method_with_requirements(
                "contains",
                Receiver::Shared,
                vec![Parameter::new("value", TypeRef::shared_ref(t.clone()))],
                TypeRef::Bool,
                vec![requirement(T, "::core::cmp::PartialEq")],
            ),
            method_with_requirements(
                "clone",
                Receiver::Shared,
                vec![],
                list_t,
                vec![requirement(T, "::core::clone::Clone")],
            ),
        ],
    }
}

#[allow(clippy::too_many_lines)]
fn queue_binding() -> NativeTypeBinding {
    let t = TypeRef::Parameter(T.to_owned());
    let queue_t = queue_of(t.clone());

    NativeTypeBinding {
        stainless_path: "rust::Queue".to_owned(),
        rust_path: "::std::collections::VecDeque".to_owned(),
        type_parameters: vec![T.to_owned()],
        error_format: None,
        callables: vec![
            constructor(
                "Queue",
                vec![],
                queue_t.clone(),
                RustLowering::AssociatedFunction {
                    rust_path: "::std::collections::VecDeque::new".to_owned(),
                },
            ),
            associated(
                "with_capacity",
                vec![Parameter::new("capacity", TypeRef::Usize)],
                queue_t.clone(),
                "::std::collections::VecDeque::with_capacity",
            ),
            method("len", Receiver::Shared, vec![], TypeRef::Usize),
            method("is_empty", Receiver::Shared, vec![], TypeRef::Bool),
            method("capacity", Receiver::Shared, vec![], TypeRef::Usize),
            method(
                "reserve",
                Receiver::Mutable,
                vec![Parameter::new("additional", TypeRef::Usize)],
                TypeRef::Void,
            ),
            method(
                "reserve_exact",
                Receiver::Mutable,
                vec![Parameter::new("additional", TypeRef::Usize)],
                TypeRef::Void,
            ),
            method("shrink_to_fit", Receiver::Mutable, vec![], TypeRef::Void),
            method(
                "shrink_to",
                Receiver::Mutable,
                vec![Parameter::new("minimum_capacity", TypeRef::Usize)],
                TypeRef::Void,
            ),
            method("clear", Receiver::Mutable, vec![], TypeRef::Void),
            method(
                "truncate",
                Receiver::Mutable,
                vec![Parameter::new("length", TypeRef::Usize)],
                TypeRef::Void,
            ),
            method(
                "push_front",
                Receiver::Mutable,
                vec![Parameter::new("value", t.clone())],
                TypeRef::Void,
            ),
            method(
                "push_back",
                Receiver::Mutable,
                vec![Parameter::new("value", t.clone())],
                TypeRef::Void,
            ),
            method("pop_front", Receiver::Mutable, vec![], option_of(t.clone())),
            method("pop_back", Receiver::Mutable, vec![], option_of(t.clone())),
            method(
                "insert",
                Receiver::Mutable,
                vec![
                    Parameter::new("index", TypeRef::Usize),
                    Parameter::new("value", t.clone()),
                ],
                TypeRef::Void,
            ),
            method(
                "remove",
                Receiver::Mutable,
                vec![Parameter::new("index", TypeRef::Usize)],
                option_of(t.clone()),
            ),
            method(
                "swap_remove_front",
                Receiver::Mutable,
                vec![Parameter::new("index", TypeRef::Usize)],
                option_of(t.clone()),
            ),
            method(
                "swap_remove_back",
                Receiver::Mutable,
                vec![Parameter::new("index", TypeRef::Usize)],
                option_of(t.clone()),
            ),
            method(
                "append",
                Receiver::Mutable,
                vec![Parameter::new(
                    "other",
                    TypeRef::mutable_ref(queue_t.clone()),
                )],
                TypeRef::Void,
            ),
            method(
                "rotate_left",
                Receiver::Mutable,
                vec![Parameter::new("middle", TypeRef::Usize)],
                TypeRef::Void,
            ),
            method(
                "rotate_right",
                Receiver::Mutable,
                vec![Parameter::new("middle", TypeRef::Usize)],
                TypeRef::Void,
            ),
            method_with_requirements(
                "contains",
                Receiver::Shared,
                vec![Parameter::new("value", TypeRef::shared_ref(t.clone()))],
                TypeRef::Bool,
                vec![requirement(T, "::core::cmp::PartialEq")],
            ),
            method_with_requirements(
                "clone",
                Receiver::Shared,
                vec![],
                queue_t,
                vec![requirement(T, "::core::clone::Clone")],
            ),
        ],
    }
}

#[allow(clippy::too_many_lines)]
fn map_binding() -> NativeTypeBinding {
    let k = TypeRef::Parameter(K.to_owned());
    let v = TypeRef::Parameter(V.to_owned());
    let map_t = map_of(k.clone(), v.clone());
    let key_ord = || vec![requirement(K, "::core::cmp::Ord")];
    let with_value = TypeRef::callback(
        CallbackKind::FnOnce,
        CallbackEscape::Call,
        vec![TypeRef::shared_ref(v.clone())],
        TypeRef::Void,
    );
    let with_value_mut = TypeRef::callback(
        CallbackKind::FnOnce,
        CallbackEscape::Call,
        vec![TypeRef::mutable_ref(v.clone())],
        TypeRef::Void,
    );
    let with_entry = TypeRef::callback(
        CallbackKind::FnOnce,
        CallbackEscape::Call,
        vec![
            TypeRef::shared_ref(k.clone()),
            TypeRef::shared_ref(v.clone()),
        ],
        TypeRef::Void,
    );
    let with_entries = TypeRef::callback(
        CallbackKind::FnMut,
        CallbackEscape::Call,
        vec![
            TypeRef::shared_ref(k.clone()),
            TypeRef::shared_ref(v.clone()),
        ],
        TypeRef::Void,
    );
    let retain_entry = TypeRef::callback(
        CallbackKind::FnMut,
        CallbackEscape::Call,
        vec![
            TypeRef::shared_ref(k.clone()),
            TypeRef::shared_ref(v.clone()),
        ],
        TypeRef::Bool,
    );
    let retain_key = TypeRef::callback(
        CallbackKind::FnMut,
        CallbackEscape::Call,
        vec![TypeRef::shared_ref(k.clone())],
        TypeRef::Bool,
    );

    NativeTypeBinding {
        stainless_path: "rust::Map".to_owned(),
        rust_path: "::std::collections::BTreeMap".to_owned(),
        type_parameters: vec![K.to_owned(), V.to_owned()],
        error_format: None,
        callables: vec![
            constructor(
                "Map",
                vec![],
                map_t.clone(),
                RustLowering::AssociatedFunction {
                    rust_path: "::std::collections::BTreeMap::new".to_owned(),
                },
            ),
            method("len", Receiver::Shared, vec![], TypeRef::Usize),
            method("is_empty", Receiver::Shared, vec![], TypeRef::Bool),
            method("clear", Receiver::Mutable, vec![], TypeRef::Void),
            method_with_requirements(
                "insert",
                Receiver::Mutable,
                vec![
                    Parameter::new("key", k.clone()),
                    Parameter::new("value", v.clone()),
                ],
                option_of(v.clone()),
                key_ord(),
            ),
            method_with_requirements(
                "remove",
                Receiver::Mutable,
                vec![Parameter::new("key", TypeRef::shared_ref(k.clone()))],
                option_of(v.clone()),
                key_ord(),
            ),
            method_with_requirements(
                "contains_key",
                Receiver::Shared,
                vec![Parameter::new("key", TypeRef::shared_ref(k.clone()))],
                TypeRef::Bool,
                key_ord(),
            ),
            CallableBinding {
                source_name: "get".to_owned(),
                style: CallStyle::Method,
                receiver: Some(Receiver::Shared),
                parameters: vec![Parameter::new("key", TypeRef::shared_ref(k.clone()))],
                is_async: false,
                return_type: option_of(v.clone()),
                rust_result_error: None,
                return_borrow: None,
                requirements: vec![
                    requirement(K, "::core::cmp::Ord"),
                    requirement(V, "::core::clone::Clone"),
                ],
                lowering: RustLowering::FunctionWithReceiver {
                    rust_path: "::stainless_runtime::btree_map_get".to_owned(),
                },
            },
            CallableBinding {
                source_name: "with".to_owned(),
                style: CallStyle::Method,
                receiver: Some(Receiver::Shared),
                parameters: vec![
                    Parameter::new("key", TypeRef::shared_ref(k.clone())),
                    Parameter::new("callback", with_value),
                ],
                is_async: false,
                return_type: TypeRef::Bool,
                rust_result_error: None,
                return_borrow: None,
                requirements: key_ord(),
                lowering: RustLowering::FunctionWithReceiver {
                    rust_path: "::stainless_runtime::btree_map_with".to_owned(),
                },
            },
            CallableBinding {
                source_name: "with_mut".to_owned(),
                style: CallStyle::Method,
                receiver: Some(Receiver::Mutable),
                parameters: vec![
                    Parameter::new("key", TypeRef::shared_ref(k.clone())),
                    Parameter::new("callback", with_value_mut),
                ],
                is_async: false,
                return_type: TypeRef::Bool,
                rust_result_error: None,
                return_borrow: None,
                requirements: key_ord(),
                lowering: RustLowering::FunctionWithReceiver {
                    rust_path: "::stainless_runtime::btree_map_with_mut".to_owned(),
                },
            },
            CallableBinding {
                source_name: "with_range".to_owned(),
                style: CallStyle::Method,
                receiver: Some(Receiver::Shared),
                parameters: vec![
                    Parameter::new("lower", TypeRef::shared_ref(k.clone())),
                    Parameter::new("upper", TypeRef::shared_ref(k.clone())),
                    Parameter::new("callback", with_entries),
                ],
                is_async: false,
                return_type: TypeRef::Usize,
                rust_result_error: None,
                return_borrow: None,
                requirements: key_ord(),
                lowering: RustLowering::FunctionWithReceiver {
                    rust_path: "::stainless_runtime::btree_map_with_range".to_owned(),
                },
            },
            CallableBinding {
                source_name: "with_first_after".to_owned(),
                style: CallStyle::Method,
                receiver: Some(Receiver::Shared),
                parameters: vec![
                    Parameter::new("lower", TypeRef::shared_ref(k.clone())),
                    Parameter::new("upper", TypeRef::shared_ref(k.clone())),
                    Parameter::new("callback", with_entry.clone()),
                ],
                is_async: false,
                return_type: TypeRef::Bool,
                rust_result_error: None,
                return_borrow: None,
                requirements: key_ord(),
                lowering: RustLowering::FunctionWithReceiver {
                    rust_path: "::stainless_runtime::btree_map_with_first_after".to_owned(),
                },
            },
            CallableBinding {
                source_name: "with_first_in_range".to_owned(),
                style: CallStyle::Method,
                receiver: Some(Receiver::Shared),
                parameters: vec![
                    Parameter::new("lower", TypeRef::shared_ref(k.clone())),
                    Parameter::new("upper", TypeRef::shared_ref(k.clone())),
                    Parameter::new("callback", with_entry.clone()),
                ],
                is_async: false,
                return_type: TypeRef::Bool,
                rust_result_error: None,
                return_borrow: None,
                requirements: key_ord(),
                lowering: RustLowering::FunctionWithReceiver {
                    rust_path: "::stainless_runtime::btree_map_with_first_in_range".to_owned(),
                },
            },
            CallableBinding {
                source_name: "with_last_before".to_owned(),
                style: CallStyle::Method,
                receiver: Some(Receiver::Shared),
                parameters: vec![
                    Parameter::new("lower", TypeRef::shared_ref(k.clone())),
                    Parameter::new("upper", TypeRef::shared_ref(k.clone())),
                    Parameter::new("callback", with_entry.clone()),
                ],
                is_async: false,
                return_type: TypeRef::Bool,
                rust_result_error: None,
                return_borrow: None,
                requirements: key_ord(),
                lowering: RustLowering::FunctionWithReceiver {
                    rust_path: "::stainless_runtime::btree_map_with_last_before".to_owned(),
                },
            },
            CallableBinding {
                source_name: "with_last_in_range".to_owned(),
                style: CallStyle::Method,
                receiver: Some(Receiver::Shared),
                parameters: vec![
                    Parameter::new("lower", TypeRef::shared_ref(k.clone())),
                    Parameter::new("upper", TypeRef::shared_ref(k)),
                    Parameter::new("callback", with_entry),
                ],
                is_async: false,
                return_type: TypeRef::Bool,
                rust_result_error: None,
                return_borrow: None,
                requirements: key_ord(),
                lowering: RustLowering::FunctionWithReceiver {
                    rust_path: "::stainless_runtime::btree_map_with_last_in_range".to_owned(),
                },
            },
            CallableBinding {
                source_name: "retain".to_owned(),
                style: CallStyle::Method,
                receiver: Some(Receiver::Mutable),
                parameters: vec![Parameter::new("predicate", retain_entry)],
                is_async: false,
                return_type: TypeRef::Void,
                rust_result_error: None,
                return_borrow: None,
                requirements: key_ord(),
                lowering: RustLowering::FunctionWithReceiver {
                    rust_path: "::stainless_runtime::btree_map_retain".to_owned(),
                },
            },
            CallableBinding {
                source_name: "retain".to_owned(),
                style: CallStyle::Method,
                receiver: Some(Receiver::Mutable),
                parameters: vec![Parameter::new("predicate", retain_key)],
                is_async: false,
                return_type: TypeRef::Void,
                rust_result_error: None,
                return_borrow: None,
                requirements: key_ord(),
                lowering: RustLowering::FunctionWithReceiver {
                    rust_path: "::stainless_runtime::btree_map_retain_keys".to_owned(),
                },
            },
            method_with_requirements(
                "append",
                Receiver::Mutable,
                vec![Parameter::new("other", TypeRef::mutable_ref(map_t.clone()))],
                TypeRef::Void,
                key_ord(),
            ),
            method_with_requirements(
                "clone",
                Receiver::Shared,
                vec![],
                map_t,
                vec![
                    requirement(K, "::core::clone::Clone"),
                    requirement(V, "::core::clone::Clone"),
                ],
            ),
        ],
    }
}

#[allow(clippy::too_many_lines)]
fn multimap_binding() -> NativeTypeBinding {
    let k = TypeRef::Parameter(K.to_owned());
    let v = TypeRef::Parameter(V.to_owned());
    let multimap_t = multimap_of(k.clone(), v.clone());
    let key_ord = || vec![requirement(K, "::core::cmp::Ord")];
    let with_value = TypeRef::callback(
        CallbackKind::FnMut,
        CallbackEscape::Call,
        vec![TypeRef::shared_ref(v.clone())],
        TypeRef::Void,
    );
    let with_value_mut = TypeRef::callback(
        CallbackKind::FnMut,
        CallbackEscape::Call,
        vec![TypeRef::mutable_ref(v.clone())],
        TypeRef::Void,
    );
    let retain_pair = TypeRef::callback(
        CallbackKind::FnMut,
        CallbackEscape::Call,
        vec![
            TypeRef::shared_ref(k.clone()),
            TypeRef::shared_ref(v.clone()),
        ],
        TypeRef::Bool,
    );

    NativeTypeBinding {
        stainless_path: "rust::MultiMap".to_owned(),
        rust_path: "::stainless_runtime::MultiMap".to_owned(),
        type_parameters: vec![K.to_owned(), V.to_owned()],
        error_format: None,
        callables: vec![
            constructor(
                "MultiMap",
                vec![],
                multimap_t.clone(),
                RustLowering::AssociatedFunction {
                    rust_path: "::stainless_runtime::MultiMap::new".to_owned(),
                },
            ),
            method("len", Receiver::Shared, vec![], TypeRef::Usize),
            method("key_len", Receiver::Shared, vec![], TypeRef::Usize),
            method("is_empty", Receiver::Shared, vec![], TypeRef::Bool),
            method("clear", Receiver::Mutable, vec![], TypeRef::Void),
            method_with_requirements(
                "insert",
                Receiver::Mutable,
                vec![
                    Parameter::new("key", k.clone()),
                    Parameter::new("value", v.clone()),
                ],
                TypeRef::Void,
                key_ord(),
            ),
            method_with_requirements(
                "contains_key",
                Receiver::Shared,
                vec![Parameter::new("key", TypeRef::shared_ref(k.clone()))],
                TypeRef::Bool,
                key_ord(),
            ),
            method_with_requirements(
                "with",
                Receiver::Shared,
                vec![
                    Parameter::new("key", TypeRef::shared_ref(k.clone())),
                    Parameter::new("callback", with_value),
                ],
                TypeRef::Usize,
                key_ord(),
            ),
            method_with_requirements(
                "with_mut",
                Receiver::Mutable,
                vec![
                    Parameter::new("key", TypeRef::shared_ref(k.clone())),
                    Parameter::new("callback", with_value_mut),
                ],
                TypeRef::Usize,
                key_ord(),
            ),
            method_with_requirements(
                "remove",
                Receiver::Mutable,
                vec![
                    Parameter::new("key", TypeRef::shared_ref(k.clone())),
                    Parameter::new("value", TypeRef::shared_ref(v.clone())),
                ],
                TypeRef::Bool,
                vec![
                    requirement(K, "::core::cmp::Ord"),
                    requirement(V, "::core::cmp::PartialEq"),
                ],
            ),
            method_with_requirements(
                "remove_all",
                Receiver::Mutable,
                vec![Parameter::new("key", TypeRef::shared_ref(k))],
                TypeRef::Usize,
                key_ord(),
            ),
            method_with_requirements(
                "retain",
                Receiver::Mutable,
                vec![Parameter::new("predicate", retain_pair)],
                TypeRef::Void,
                key_ord(),
            ),
            method_with_requirements(
                "clone",
                Receiver::Shared,
                vec![],
                multimap_t,
                vec![
                    requirement(K, "::core::clone::Clone"),
                    requirement(V, "::core::clone::Clone"),
                ],
            ),
        ],
    }
}

fn set_binding() -> NativeTypeBinding {
    let t = TypeRef::Parameter(T.to_owned());
    let set_t = set_of(t.clone());
    let element_ord = || vec![requirement(T, "::core::cmp::Ord")];

    NativeTypeBinding {
        stainless_path: "rust::Set".to_owned(),
        rust_path: "::std::collections::BTreeSet".to_owned(),
        type_parameters: vec![T.to_owned()],
        error_format: None,
        callables: vec![
            constructor(
                "Set",
                vec![],
                set_t.clone(),
                RustLowering::AssociatedFunction {
                    rust_path: "::std::collections::BTreeSet::new".to_owned(),
                },
            ),
            method("len", Receiver::Shared, vec![], TypeRef::Usize),
            method("is_empty", Receiver::Shared, vec![], TypeRef::Bool),
            method("clear", Receiver::Mutable, vec![], TypeRef::Void),
            method_with_requirements(
                "insert",
                Receiver::Mutable,
                vec![Parameter::new("value", t.clone())],
                TypeRef::Bool,
                element_ord(),
            ),
            method_with_requirements(
                "replace",
                Receiver::Mutable,
                vec![Parameter::new("value", t.clone())],
                option_of(t.clone()),
                element_ord(),
            ),
            method_with_requirements(
                "remove",
                Receiver::Mutable,
                vec![Parameter::new("value", TypeRef::shared_ref(t.clone()))],
                TypeRef::Bool,
                element_ord(),
            ),
            method_with_requirements(
                "take",
                Receiver::Mutable,
                vec![Parameter::new("value", TypeRef::shared_ref(t.clone()))],
                option_of(t.clone()),
                element_ord(),
            ),
            method_with_requirements(
                "contains",
                Receiver::Shared,
                vec![Parameter::new("value", TypeRef::shared_ref(t))],
                TypeRef::Bool,
                element_ord(),
            ),
            method_with_requirements(
                "append",
                Receiver::Mutable,
                vec![Parameter::new("other", TypeRef::mutable_ref(set_t.clone()))],
                TypeRef::Void,
                element_ord(),
            ),
            method_with_requirements(
                "clone",
                Receiver::Shared,
                vec![],
                set_t,
                vec![requirement(T, "::core::clone::Clone")],
            ),
        ],
    }
}

fn vec_construction(vec_t: &TypeRef) -> Vec<CallableBinding> {
    vec![
        constructor(
            "Vec",
            vec![],
            vec_t.clone(),
            RustLowering::AssociatedFunction {
                rust_path: "::std::vec::Vec::new".to_owned(),
            },
        ),
        associated(
            "with_capacity",
            vec![Parameter::new("capacity", TypeRef::Usize)],
            vec_t.clone(),
            "::std::vec::Vec::with_capacity",
        ),
    ]
}

fn vec_capacity_methods() -> Vec<CallableBinding> {
    vec![
        method("len", Receiver::Shared, vec![], TypeRef::Usize),
        renamed_method("empty", "is_empty", Receiver::Shared, vec![], TypeRef::Bool),
        method("capacity", Receiver::Shared, vec![], TypeRef::Usize),
        method(
            "reserve",
            Receiver::Mutable,
            vec![Parameter::new("additional", TypeRef::Usize)],
            TypeRef::Void,
        ),
        method(
            "reserve_exact",
            Receiver::Mutable,
            vec![Parameter::new("additional", TypeRef::Usize)],
            TypeRef::Void,
        ),
        method(
            "shrink_to",
            Receiver::Mutable,
            vec![Parameter::new("minimum_capacity", TypeRef::Usize)],
            TypeRef::Void,
        ),
        method("shrink_to_fit", Receiver::Mutable, vec![], TypeRef::Void),
    ]
}

fn vec_element_methods(t: &TypeRef, vec_t: &TypeRef) -> Vec<CallableBinding> {
    vec![
        method(
            "push",
            Receiver::Mutable,
            vec![Parameter::new("value", t.clone())],
            TypeRef::Void,
        ),
        method("pop", Receiver::Mutable, vec![], option_of(t.clone())),
        method("clear", Receiver::Mutable, vec![], TypeRef::Void),
        method(
            "truncate",
            Receiver::Mutable,
            vec![Parameter::new("length", TypeRef::Usize)],
            TypeRef::Void,
        ),
        method(
            "insert",
            Receiver::Mutable,
            vec![
                Parameter::new("index", TypeRef::Usize),
                Parameter::new("value", t.clone()),
            ],
            TypeRef::Void,
        ),
        method(
            "remove",
            Receiver::Mutable,
            vec![Parameter::new("index", TypeRef::Usize)],
            t.clone(),
        ),
        method(
            "swap_remove",
            Receiver::Mutable,
            vec![Parameter::new("index", TypeRef::Usize)],
            t.clone(),
        ),
        method(
            "append",
            Receiver::Mutable,
            vec![Parameter::new("other", TypeRef::mutable_ref(vec_t.clone()))],
            TypeRef::Void,
        ),
        method("reverse", Receiver::Mutable, vec![], TypeRef::Void),
    ]
}

fn vec_trait_methods(t: &TypeRef, vec_t: &TypeRef) -> Vec<CallableBinding> {
    vec![
        method_with_requirements(
            "extend_from_slice",
            Receiver::Mutable,
            vec![Parameter::new("other", TypeRef::shared_ref(vec_t.clone()))],
            TypeRef::Void,
            vec![requirement(T, "::core::clone::Clone")],
        ),
        method_with_requirements(
            "clone",
            Receiver::Shared,
            vec![],
            vec_t.clone(),
            vec![requirement(T, "::core::clone::Clone")],
        ),
        method_with_requirements(
            "contains",
            Receiver::Shared,
            vec![Parameter::new("value", TypeRef::shared_ref(t.clone()))],
            TypeRef::Bool,
            vec![requirement(T, "::core::cmp::PartialEq")],
        ),
        method_with_requirements(
            "sort",
            Receiver::Mutable,
            vec![],
            TypeRef::Void,
            vec![requirement(T, "::core::cmp::Ord")],
        ),
        method_with_requirements(
            "dedup",
            Receiver::Mutable,
            vec![],
            TypeRef::Void,
            vec![requirement(T, "::core::cmp::PartialEq")],
        ),
    ]
}

fn string_binding() -> NativeTypeBinding {
    let string = string_type();
    let string_ref = TypeRef::shared_ref(string.clone());
    let mut callables = string_construction(&string, &string_ref);
    callables.extend(string_capacity_methods());
    callables.extend(string_mutation_methods(&string_ref));
    callables.extend(string_query_methods(&string, &string_ref));

    NativeTypeBinding {
        stainless_path: "rust::String".to_owned(),
        rust_path: "::std::string::String".to_owned(),
        type_parameters: vec![],
        error_format: None,
        callables,
    }
}

fn string_from_utf8_error_binding() -> NativeTypeBinding {
    NativeTypeBinding {
        stainless_path: "rust::std::string::FromUtf8Error".to_owned(),
        rust_path: "::std::string::FromUtf8Error".to_owned(),
        type_parameters: vec![],
        error_format: Some(NativeErrorFormat::Display),
        callables: vec![],
    }
}

fn string_construction(string: &TypeRef, string_ref: &TypeRef) -> Vec<CallableBinding> {
    vec![
        constructor(
            "String",
            vec![],
            string.clone(),
            RustLowering::AssociatedFunction {
                rust_path: "::std::string::String::new".to_owned(),
            },
        ),
        constructor(
            "String",
            vec![Parameter::new("value", string_ref.clone())],
            string.clone(),
            RustLowering::CloneArgument { index: 0 },
        ),
        associated(
            "with_capacity",
            vec![Parameter::new("capacity", TypeRef::Usize)],
            string.clone(),
            "::std::string::String::with_capacity",
        ),
        fallible_associated(
            "from_utf8",
            vec![Parameter::new("bytes", vec_of(TypeRef::U8))],
            string.clone(),
            TypeRef::native("rust::std::string::FromUtf8Error", vec![]),
            "::std::string::String::from_utf8",
        ),
        method("clone", Receiver::Shared, vec![], string.clone()),
        method("into_bytes", Receiver::Value, vec![], vec_of(TypeRef::U8)),
    ]
}

fn string_capacity_methods() -> Vec<CallableBinding> {
    vec![
        method("len", Receiver::Shared, vec![], TypeRef::Usize),
        renamed_method("empty", "is_empty", Receiver::Shared, vec![], TypeRef::Bool),
        method("capacity", Receiver::Shared, vec![], TypeRef::Usize),
        method(
            "reserve",
            Receiver::Mutable,
            vec![Parameter::new("additional", TypeRef::Usize)],
            TypeRef::Void,
        ),
        method(
            "reserve_exact",
            Receiver::Mutable,
            vec![Parameter::new("additional", TypeRef::Usize)],
            TypeRef::Void,
        ),
        method(
            "shrink_to",
            Receiver::Mutable,
            vec![Parameter::new("minimum_capacity", TypeRef::Usize)],
            TypeRef::Void,
        ),
        method("shrink_to_fit", Receiver::Mutable, vec![], TypeRef::Void),
        method(
            "truncate",
            Receiver::Mutable,
            vec![Parameter::new("length", TypeRef::Usize)],
            TypeRef::Void,
        ),
    ]
}

fn string_mutation_methods(string_ref: &TypeRef) -> Vec<CallableBinding> {
    vec![
        method("clear", Receiver::Mutable, vec![], TypeRef::Void),
        method(
            "push",
            Receiver::Mutable,
            vec![Parameter::new("value", TypeRef::Char)],
            TypeRef::Void,
        ),
        string_ref_method(
            "push_str",
            Receiver::Mutable,
            vec![("value", string_ref.clone())],
            TypeRef::Void,
        ),
        method("pop", Receiver::Mutable, vec![], option_of(TypeRef::Char)),
        method(
            "insert",
            Receiver::Mutable,
            vec![
                Parameter::new("index", TypeRef::Usize),
                Parameter::new("value", TypeRef::Char),
            ],
            TypeRef::Void,
        ),
        string_ref_method(
            "insert_str",
            Receiver::Mutable,
            vec![("index", TypeRef::Usize), ("value", string_ref.clone())],
            TypeRef::Void,
        ),
        method(
            "remove",
            Receiver::Mutable,
            vec![Parameter::new("index", TypeRef::Usize)],
            TypeRef::Char,
        ),
        method(
            "make_ascii_lowercase",
            Receiver::Mutable,
            vec![],
            TypeRef::Void,
        ),
        method(
            "make_ascii_uppercase",
            Receiver::Mutable,
            vec![],
            TypeRef::Void,
        ),
    ]
}

fn string_query_methods(string: &TypeRef, string_ref: &TypeRef) -> Vec<CallableBinding> {
    vec![
        method("is_ascii", Receiver::Shared, vec![], TypeRef::Bool),
        string_ref_method(
            "contains",
            Receiver::Shared,
            vec![("pattern", string_ref.clone())],
            TypeRef::Bool,
        ),
        string_ref_method(
            "starts_with",
            Receiver::Shared,
            vec![("pattern", string_ref.clone())],
            TypeRef::Bool,
        ),
        string_ref_method(
            "ends_with",
            Receiver::Shared,
            vec![("pattern", string_ref.clone())],
            TypeRef::Bool,
        ),
        string_ref_method(
            "eq_ignore_ascii_case",
            Receiver::Shared,
            vec![("other", string_ref.clone())],
            TypeRef::Bool,
        ),
        string_ref_method(
            "replace",
            Receiver::Shared,
            vec![("from", string_ref.clone()), ("to", string_ref.clone())],
            string.clone(),
        ),
        method(
            "repeat",
            Receiver::Shared,
            vec![Parameter::new("count", TypeRef::Usize)],
            string.clone(),
        ),
        method("to_lowercase", Receiver::Shared, vec![], string.clone()),
        method("to_uppercase", Receiver::Shared, vec![], string.clone()),
    ]
}

fn constructor(
    source_name: &'static str,
    parameters: Vec<Parameter>,
    return_type: TypeRef,
    lowering: RustLowering,
) -> CallableBinding {
    CallableBinding {
        source_name: source_name.to_owned(),
        style: CallStyle::Constructor,
        receiver: None,
        parameters,
        is_async: false,
        return_type,
        rust_result_error: None,
        return_borrow: None,
        requirements: vec![],
        lowering,
    }
}

fn associated(
    source_name: &'static str,
    parameters: Vec<Parameter>,
    return_type: TypeRef,
    rust_path: &str,
) -> CallableBinding {
    CallableBinding {
        source_name: source_name.to_owned(),
        style: CallStyle::AssociatedFunction,
        receiver: None,
        parameters,
        is_async: false,
        return_type,
        rust_result_error: None,
        return_borrow: None,
        requirements: vec![],
        lowering: RustLowering::AssociatedFunction {
            rust_path: rust_path.to_owned(),
        },
    }
}

fn fallible_associated(
    source_name: &'static str,
    parameters: Vec<Parameter>,
    return_type: TypeRef,
    error_type: TypeRef,
    rust_path: &str,
) -> CallableBinding {
    CallableBinding {
        source_name: source_name.to_owned(),
        style: CallStyle::AssociatedFunction,
        receiver: None,
        parameters,
        is_async: false,
        return_type,
        rust_result_error: Some(error_type),
        return_borrow: None,
        requirements: vec![],
        lowering: RustLowering::AssociatedFunction {
            rust_path: rust_path.to_owned(),
        },
    }
}

fn method(
    source_name: &'static str,
    receiver: Receiver,
    parameters: Vec<Parameter>,
    return_type: TypeRef,
) -> CallableBinding {
    method_with_requirements(source_name, receiver, parameters, return_type, vec![])
}

fn method_with_requirements(
    source_name: &'static str,
    receiver: Receiver,
    parameters: Vec<Parameter>,
    return_type: TypeRef,
    requirements: Vec<TraitRequirement>,
) -> CallableBinding {
    renamed_method_with_requirements(
        source_name,
        source_name,
        receiver,
        parameters,
        return_type,
        requirements,
    )
}

fn renamed_method(
    source_name: &'static str,
    rust_name: &'static str,
    receiver: Receiver,
    parameters: Vec<Parameter>,
    return_type: TypeRef,
) -> CallableBinding {
    renamed_method_with_requirements(
        source_name,
        rust_name,
        receiver,
        parameters,
        return_type,
        vec![],
    )
}

fn renamed_method_with_requirements(
    source_name: &'static str,
    rust_name: &'static str,
    receiver: Receiver,
    parameters: Vec<Parameter>,
    return_type: TypeRef,
    requirements: Vec<TraitRequirement>,
) -> CallableBinding {
    CallableBinding {
        source_name: source_name.to_owned(),
        style: CallStyle::Method,
        receiver: Some(receiver),
        parameters,
        is_async: false,
        return_type,
        rust_result_error: None,
        return_borrow: None,
        requirements,
        lowering: RustLowering::Method {
            rust_name: rust_name.to_owned(),
        },
    }
}

fn fallible_method(
    source_name: &'static str,
    rust_name: &'static str,
    receiver: Receiver,
    parameters: Vec<Parameter>,
    return_type: TypeRef,
    error_type: TypeRef,
) -> CallableBinding {
    CallableBinding {
        source_name: source_name.to_owned(),
        style: CallStyle::Method,
        receiver: Some(receiver),
        parameters,
        is_async: false,
        return_type,
        rust_result_error: Some(error_type),
        return_borrow: None,
        requirements: vec![],
        lowering: RustLowering::Method {
            rust_name: rust_name.to_owned(),
        },
    }
}

fn string_ref_method(
    source_name: &'static str,
    receiver: Receiver,
    parameters: Vec<(&'static str, TypeRef)>,
    return_type: TypeRef,
) -> CallableBinding {
    method(
        source_name,
        receiver,
        parameters
            .into_iter()
            .map(|(name, ty)| {
                let adaptation = if ty == TypeRef::shared_ref(string_type()) {
                    ArgumentAdaptation::StringRefToStr
                } else {
                    ArgumentAdaptation::Identity
                };
                Parameter::adapted(name, ty, adaptation)
            })
            .collect(),
        return_type,
    )
}

fn requirement(parameter: &'static str, rust_trait: &'static str) -> TraitRequirement {
    TraitRequirement {
        parameter: parameter.to_owned(),
        rust_trait: rust_trait.to_owned(),
    }
}

fn string_type() -> TypeRef {
    TypeRef::native("rust::String", vec![])
}

fn vec_of(element: TypeRef) -> TypeRef {
    TypeRef::native("rust::Vec", vec![element])
}

fn list_of(element: TypeRef) -> TypeRef {
    TypeRef::native("rust::List", vec![element])
}

fn queue_of(element: TypeRef) -> TypeRef {
    TypeRef::native("rust::Queue", vec![element])
}

fn map_of(key: TypeRef, value: TypeRef) -> TypeRef {
    TypeRef::native("rust::Map", vec![key, value])
}

fn multimap_of(key: TypeRef, value: TypeRef) -> TypeRef {
    TypeRef::native("rust::MultiMap", vec![key, value])
}

fn set_of(element: TypeRef) -> TypeRef {
    TypeRef::native("rust::Set", vec![element])
}

fn option_of(value: TypeRef) -> TypeRef {
    TypeRef::native("rust::Option", vec![value])
}

fn option_binding() -> NativeTypeBinding {
    let t = TypeRef::Parameter(T.to_owned());
    let option = option_of(t.clone());

    NativeTypeBinding {
        stainless_path: "rust::Option".to_owned(),
        rust_path: "::core::option::Option".to_owned(),
        type_parameters: vec![T.to_owned()],
        error_format: None,
        callables: vec![
            constructor(
                "Option",
                vec![],
                option.clone(),
                RustLowering::AssociatedFunction {
                    rust_path: "::core::option::Option::default".to_owned(),
                },
            ),
            constructor(
                "Option",
                vec![Parameter::new("value", t.clone())],
                option.clone(),
                RustLowering::AssociatedFunction {
                    rust_path: "::core::option::Option::Some".to_owned(),
                },
            ),
            renamed_method(
                "has_value",
                "is_some",
                Receiver::Shared,
                vec![],
                TypeRef::Bool,
            ),
            CallableBinding {
                source_name: "value_or".to_owned(),
                style: CallStyle::Method,
                receiver: Some(Receiver::Shared),
                parameters: vec![Parameter::new("fallback", t.clone())],
                is_async: false,
                return_type: t.clone(),
                rust_result_error: None,
                return_borrow: None,
                requirements: vec![requirement(T, "::core::clone::Clone")],
                lowering: RustLowering::ClonedReceiverMethod {
                    rust_name: "unwrap_or".to_owned(),
                },
            },
            method_with_requirements(
                "clone",
                Receiver::Shared,
                vec![],
                option,
                vec![requirement(T, "::core::clone::Clone")],
            ),
        ],
    }
}
