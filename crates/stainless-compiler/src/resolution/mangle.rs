use crate::interop::TypeRef;

pub(super) fn function_name(path: &[String], parameters: &[TypeRef]) -> String {
    let mut output = format!("__stainless_v1_f_{}", path.len());
    for segment in path {
        output.push('_');
        output.push_str(&segment.len().to_string());
        output.push('_');
        output.push_str(segment);
    }
    output.push_str("__p_");
    output.push_str(&parameters.len().to_string());
    for parameter in parameters {
        output.push('_');
        encode_type(canonical(parameter), &mut output);
    }
    output
}

fn encode_type(ty: &TypeRef, output: &mut String) {
    if let Some(spelling) = primitive_spelling(ty) {
        output.push_str(spelling);
        return;
    }
    match ty {
        TypeRef::Struct { path } => {
            output.push_str("s_");
            encode_segments(path, output);
        }
        TypeRef::Native { path, arguments } if arguments.is_empty() => {
            output.push_str("n_");
            encode_path(path, output);
        }
        TypeRef::Native { path, arguments } => {
            output.push_str("g_");
            encode_path(path, output);
            output.push_str("_a_");
            output.push_str(&arguments.len().to_string());
            for argument in arguments {
                output.push('_');
                encode_type(canonical(argument), output);
            }
        }
        TypeRef::Function(function) => {
            output.push_str(match function.kind {
                crate::interop::StoredFunctionKind::Shared => "fn_",
                crate::interop::StoredFunctionKind::Mutable => "fnmut_",
            });
            output.push_str(&function.parameters.len().to_string());
            for parameter in &function.parameters {
                output.push('_');
                encode_signature_type(parameter, output);
            }
            output.push_str("_r_");
            encode_type(&function.return_type, output);
        }
        TypeRef::Reference { target, .. } => encode_type(target, output),
        TypeRef::Parameter(name) => {
            output.push_str("t_");
            output.push_str(&name.len().to_string());
            output.push('_');
            output.push_str(name);
        }
        TypeRef::Error => output.push_str("error"),
        _ => output.push_str("unknown"),
    }
}

fn encode_signature_type(ty: &TypeRef, output: &mut String) {
    if let TypeRef::Reference { mutable, target } = ty {
        output.push_str(if *mutable { "mr_" } else { "sr_" });
        encode_type(target, output);
    } else {
        encode_type(ty, output);
    }
}

fn encode_path(path: &str, output: &mut String) {
    let segments = path.split("::").collect::<Vec<_>>();
    encode_segments(&segments, output);
}

fn encode_segments<S: AsRef<str>>(segments: &[S], output: &mut String) {
    output.push_str(&segments.len().to_string());
    for segment in segments {
        let segment = segment.as_ref();
        output.push('_');
        output.push_str(&segment.len().to_string());
        output.push('_');
        output.push_str(segment);
    }
}

fn canonical(ty: &TypeRef) -> &TypeRef {
    match ty {
        TypeRef::Reference { target, .. } => target,
        _ => ty,
    }
}

fn primitive_spelling(ty: &TypeRef) -> Option<&'static str> {
    match ty {
        TypeRef::Void => Some("void"),
        TypeRef::Bool => Some("bool"),
        TypeRef::Char => Some("char"),
        TypeRef::I8 => Some("i8"),
        TypeRef::I16 => Some("i16"),
        TypeRef::I32 => Some("i32"),
        TypeRef::I64 => Some("i64"),
        TypeRef::I128 => Some("i128"),
        TypeRef::Isize => Some("isize"),
        TypeRef::U8 => Some("u8"),
        TypeRef::U16 => Some("u16"),
        TypeRef::U32 => Some("u32"),
        TypeRef::U64 => Some("u64"),
        TypeRef::U128 => Some("u128"),
        TypeRef::Usize => Some("usize"),
        TypeRef::F32 => Some("f32"),
        TypeRef::F64 => Some("f64"),
        _ => None,
    }
}
