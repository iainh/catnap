use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote};
use syn::{
    Attribute, FnArg, Ident, ItemTrait, LitStr, Pat, ReturnType, TraitItem, TraitItemFn, Type,
    parse_macro_input, spanned::Spanned,
};

/// Generates a reqwest-backed client implementation for an annotated trait.
///
/// The generated client is named `<TraitName>Client` and implements the source
/// trait. The optional `path`, `consumes`, and `produces` arguments define
/// resource-level defaults for methods.
#[proc_macro_attribute]
pub fn rest_client(attr: TokenStream, item: TokenStream) -> TokenStream {
    let args = parse_macro_input!(attr as RestClientArgs);
    let mut trait_item = parse_macro_input!(item as ItemTrait);
    let expanded = expand_rest_client(args, &mut trait_item);
    TokenStream::from(expanded)
}

/// Marks a trait method as an HTTP `GET` request.
#[proc_macro_attribute]
pub fn get(_: TokenStream, item: TokenStream) -> TokenStream {
    item
}

/// Marks a trait method as an HTTP `POST` request.
#[proc_macro_attribute]
pub fn post(_: TokenStream, item: TokenStream) -> TokenStream {
    item
}

/// Marks a trait method as an HTTP `PUT` request.
#[proc_macro_attribute]
pub fn put(_: TokenStream, item: TokenStream) -> TokenStream {
    item
}

/// Marks a trait method as an HTTP `PATCH` request.
#[proc_macro_attribute]
pub fn patch(_: TokenStream, item: TokenStream) -> TokenStream {
    item
}

/// Marks a trait method as an HTTP `DELETE` request.
#[proc_macro_attribute]
pub fn delete(_: TokenStream, item: TokenStream) -> TokenStream {
    item
}

/// Marks a trait method as an HTTP `OPTIONS` request.
#[proc_macro_attribute]
pub fn options(_: TokenStream, item: TokenStream) -> TokenStream {
    item
}

/// Marks a trait method as an HTTP `HEAD` request.
#[proc_macro_attribute]
pub fn head(_: TokenStream, item: TokenStream) -> TokenStream {
    item
}

/// Binds a method argument to a path placeholder.
#[proc_macro_attribute]
pub fn path(_: TokenStream, item: TokenStream) -> TokenStream {
    item
}

/// Binds a method argument to a query parameter.
#[proc_macro_attribute]
pub fn query(_: TokenStream, item: TokenStream) -> TokenStream {
    item
}

/// Binds a method argument to a request header.
#[proc_macro_attribute]
pub fn header(_: TokenStream, item: TokenStream) -> TokenStream {
    item
}

/// Sets the media type used to serialize the request body.
#[proc_macro_attribute]
pub fn consumes(_: TokenStream, item: TokenStream) -> TokenStream {
    item
}

/// Sets the media type used to deserialize the response body.
#[proc_macro_attribute]
pub fn produces(_: TokenStream, item: TokenStream) -> TokenStream {
    item
}

struct RestClientArgs {
    path: String,
    consumes: MediaType,
    produces: MediaType,
}

impl syn::parse::Parse for RestClientArgs {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        let mut args = Self {
            path: String::new(),
            consumes: MediaType::Json,
            produces: MediaType::Json,
        };
        if input.is_empty() {
            return Ok(args);
        }

        let mut seen_path = false;
        let mut seen_consumes = false;
        let mut seen_produces = false;

        while !input.is_empty() {
            let key: Ident = input.parse()?;
            input.parse::<syn::Token![=]>()?;
            let value: LitStr = input.parse()?;
            match key.to_string().as_str() {
                "path" => {
                    reject_duplicate_arg(&mut seen_path, &key)?;
                    args.path = validate_path_template_lit(&value)?;
                }
                "consumes" => {
                    reject_duplicate_arg(&mut seen_consumes, &key)?;
                    args.consumes = validate_media_type_lit(&value)?;
                }
                "produces" => {
                    reject_duplicate_arg(&mut seen_produces, &key)?;
                    args.produces = validate_media_type_lit(&value)?;
                }
                _ => {
                    return Err(syn::Error::new(
                        key.span(),
                        "expected `path`, `consumes`, or `produces`",
                    ));
                }
            }
            if input.is_empty() {
                break;
            }
            input.parse::<syn::Token![,]>()?;
        }
        Ok(args)
    }
}

fn reject_duplicate_arg(seen: &mut bool, key: &Ident) -> syn::Result<()> {
    if *seen {
        return Err(syn::Error::new(
            key.span(),
            format!("duplicate `{key}` argument"),
        ));
    }
    *seen = true;
    Ok(())
}

const APPLICATION_JSON: &str = "application/json";
const APPLICATION_XML: &str = "application/xml";
const TEXT_PLAIN: &str = "text/plain";
const TEXT_XML: &str = "text/xml";

#[derive(Debug, Clone, PartialEq, Eq)]
enum MediaType {
    Json,
    Xml,
    TextXml,
    TextPlain,
    Other(String),
}

impl MediaType {
    fn parse(value: &str) -> Self {
        match value {
            APPLICATION_JSON => Self::Json,
            APPLICATION_XML => Self::Xml,
            TEXT_XML => Self::TextXml,
            TEXT_PLAIN => Self::TextPlain,
            _ => Self::Other(value.to_owned()),
        }
    }

    fn as_str(&self) -> &str {
        match self {
            Self::Json => APPLICATION_JSON,
            Self::Xml => APPLICATION_XML,
            Self::TextXml => TEXT_XML,
            Self::TextPlain => TEXT_PLAIN,
            Self::Other(value) => value,
        }
    }

    fn is_xml(&self) -> bool {
        matches!(self, Self::Xml | Self::TextXml)
    }
}

fn expand_rest_client(args: RestClientArgs, trait_item: &mut ItemTrait) -> TokenStream2 {
    trait_item
        .attrs
        .push(syn::parse_quote!(#[allow(async_fn_in_trait)]));

    let trait_ident = trait_item.ident.clone();
    let client_ident = format_ident!("{trait_ident}Client");
    let vis = trait_item.vis.clone();
    let trait_config = ResourceConfig {
        path: args.path,
        consumes: args.consumes,
        produces: args.produces,
    };
    let catnap = catnap_crate_path();

    let mut errors = Vec::new();
    let impl_methods = trait_item
        .items
        .iter_mut()
        .filter_map(|item| match item {
            TraitItem::Fn(method) => match expand_method(&catnap, &trait_config, method) {
                Ok(method) => Some(method),
                Err(error) => {
                    errors.push(error.to_compile_error());
                    None
                }
            },
            _ => None,
        })
        .collect::<Vec<_>>();

    if !errors.is_empty() {
        return quote! {
            #trait_item
            #(#errors)*
        };
    }

    quote! {
        #trait_item

        #[derive(Clone, Debug)]
        #vis struct #client_ident {
            inner: #catnap::RestClient,
        }

        impl #client_ident {
            pub fn builder() -> #catnap::RestClientBuilder<Self> {
                #catnap::RestClientBuilder::new()
            }
        }

        impl #catnap::BuildFromConfig for #client_ident {
            fn build_from_config(config: #catnap::RestClientConfig) -> #catnap::Result<Self> {
                Ok(Self {
                    inner: #catnap::RestClient::from_config(config)?,
                })
            }
        }

        impl #trait_ident for #client_ident {
            #(#impl_methods)*
        }
    }
}

fn catnap_crate_path() -> TokenStream2 {
    quote!(::catnap)
}

fn expand_method(
    catnap: &TokenStream2,
    trait_config: &ResourceConfig,
    method: &mut TraitItemFn,
) -> syn::Result<TokenStream2> {
    validate_method_signature(method)?;

    let (verb, method_path) = take_http_attr(&mut method.attrs)?.ok_or_else(|| {
        syn::Error::new_spanned(
            &method.sig.ident,
            "REST client methods must have an HTTP method attribute",
        )
    })?;
    let consumes = take_media_type_attr(&mut method.attrs, "consumes")?
        .unwrap_or_else(|| trait_config.consumes.clone());
    let produces = take_media_type_attr(&mut method.attrs, "produces")?
        .unwrap_or_else(|| trait_config.produces.clone());
    let full_path = join_paths(&trait_config.path, &method_path);

    let mut path_replacements = Vec::new();
    let mut query_params = Vec::new();
    let mut header_params = Vec::new();
    let mut body_arg = None;
    let mut path_names = Vec::new();
    let path_var = format_ident!("__catnap_path");
    let request_var = format_ident!("__catnap_request");

    for input in &mut method.sig.inputs {
        let FnArg::Typed(pat_type) = input else {
            continue;
        };

        let arg_ident = pat_ident(&pat_type.pat).ok_or_else(|| {
            syn::Error::new_spanned(
                &pat_type.pat,
                "REST client method parameters must use simple identifier patterns",
            )
        })?;

        let bindings = take_parameter_attrs(&mut pat_type.attrs, &arg_ident)?;
        match bindings.as_slice() {
            [] => {
                if matches!(verb.as_str(), "GET" | "HEAD") {
                    return Err(syn::Error::new_spanned(
                        pat_type,
                        "GET and HEAD REST client methods cannot have request body arguments",
                    ));
                }
                if body_arg.replace(arg_ident).is_some() {
                    return Err(syn::Error::new_spanned(
                        pat_type,
                        "REST client methods may have at most one unannotated body argument",
                    ));
                }
            }
            [
                ParameterAttr {
                    kind: ParameterAttrKind::PathParam,
                    name,
                },
            ] => {
                path_names.push(name.clone());
                path_replacements.push(quote! {
                    #path_var = #path_var.replace(
                        concat!("{", #name, "}"),
                        &#catnap::__private::encode_path_segment(#arg_ident),
                    );
                });
            }
            [
                ParameterAttr {
                    kind: ParameterAttrKind::QueryParam,
                    name,
                },
            ] => {
                if is_query_collection(&pat_type.ty) {
                    query_params.push(quote! {
                        #request_var = #request_var.query_params(#name, #arg_ident);
                    });
                } else {
                    query_params.push(quote! {
                        #request_var = #request_var.query_param(#name, #arg_ident);
                    });
                }
            }
            [
                ParameterAttr {
                    kind: ParameterAttrKind::Header,
                    name,
                },
            ] => {
                header_params.push(quote! {
                    #request_var = #request_var.header(#name, #arg_ident);
                });
            }
            _ => {
                return Err(syn::Error::new_spanned(
                    pat_type,
                    "REST client method parameters may have only one of #[path()], #[query()], or #[header]",
                ));
            }
        }
    }

    if has_duplicate(&path_names) {
        return Err(syn::Error::new_spanned(
            &method.sig.ident,
            "REST client path parameter names must be unique",
        ));
    }

    validate_paths(&full_path, &path_names, method)?;

    let sig = method.sig.clone();
    let output = &sig.output;
    let body = body_for(catnap, &request_var, body_arg, &consumes)?;
    let sender = sender_for(catnap, &request_var, output, &produces)?;
    let produces = produces.as_str();
    let verb_ident = format_ident!("{}", verb);

    Ok(quote! {
        #sig {
            let mut #path_var = #full_path.to_owned();
            #(#path_replacements)*
            let mut #request_var = self.inner.request(#catnap::http::Method::#verb_ident, &#path_var)?;
            #request_var = #request_var.accept(#produces);
            #(#query_params)*
            #(#header_params)*
            #body
            #sender
        }
    })
}

fn validate_method_signature(method: &TraitItemFn) -> syn::Result<()> {
    if method.sig.asyncness.is_none() {
        return Err(syn::Error::new_spanned(
            &method.sig.ident,
            "REST client methods must be async",
        ));
    }

    if method.default.is_some() {
        return Err(syn::Error::new_spanned(
            &method.sig.ident,
            "REST client methods must not define a default body",
        ));
    }

    if method.sig.constness.is_some()
        || method.sig.unsafety.is_some()
        || method.sig.abi.is_some()
        || !method.sig.generics.params.is_empty()
    {
        return Err(syn::Error::new_spanned(
            &method.sig.ident,
            "REST client methods must be non-generic safe async trait methods",
        ));
    }

    let mut inputs = method.sig.inputs.iter();
    let Some(first) = inputs.next() else {
        return Err(syn::Error::new_spanned(
            &method.sig.ident,
            "REST client methods must take &self as the first parameter",
        ));
    };

    let FnArg::Receiver(receiver) = first else {
        return Err(syn::Error::new_spanned(
            first,
            "REST client methods must take &self as the first parameter",
        ));
    };

    if receiver.reference.is_none()
        || receiver.mutability.is_some()
        || !receiver.attrs.is_empty()
        || receiver.colon_token.is_some()
    {
        return Err(syn::Error::new_spanned(
            receiver,
            "REST client methods must take &self, not self or &mut self",
        ));
    }

    Ok(())
}

fn take_http_attr(attrs: &mut Vec<Attribute>) -> syn::Result<Option<(String, String)>> {
    let mut found = None;
    let mut index = 0;
    while index < attrs.len() {
        let attr = &attrs[index];
        let Some(verb) = ["get", "post", "put", "patch", "delete", "options", "head"]
            .into_iter()
            .find(|verb| is_catnap_attr(attr, verb))
        else {
            index += 1;
            continue;
        };

        if found.is_some() {
            return Err(syn::Error::new_spanned(
                attr,
                "REST client methods may have only one HTTP method attribute",
            ));
        }

        let path = parse_optional_string_arg(attr, "HTTP method attributes")?.unwrap_or_default();
        validate_path_template(&path, attr.span())?;
        attrs.remove(index);
        found = Some((verb.to_uppercase(), path));
    }
    Ok(found)
}

#[derive(Debug)]
struct ResourceConfig {
    path: String,
    consumes: MediaType,
    produces: MediaType,
}

fn take_media_type_attr(attrs: &mut Vec<Attribute>, name: &str) -> syn::Result<Option<MediaType>> {
    let matches = matching_attr_indices(attrs, name);
    let Some(first_index) = matches.first().copied() else {
        return Ok(None);
    };
    if matches.len() > 1 {
        return Err(syn::Error::new_spanned(
            &attrs[matches[1]],
            format!("REST client methods may have only one #[{name}] attribute"),
        ));
    }

    let value = validate_media_type(
        &parse_required_string_arg(&attrs[first_index], name)?,
        attrs[first_index].span(),
    )?;
    attrs.remove(first_index);
    Ok(Some(value))
}

#[derive(Debug)]
struct ParameterAttr {
    kind: ParameterAttrKind,
    name: String,
}

#[derive(Debug)]
enum ParameterAttrKind {
    PathParam,
    QueryParam,
    Header,
}

fn take_parameter_attrs(
    attrs: &mut Vec<Attribute>,
    inferred_name: &Ident,
) -> syn::Result<Vec<ParameterAttr>> {
    let mut bindings = Vec::new();
    let mut remove_indices = Vec::new();

    for (index, attr) in attrs.iter().enumerate() {
        let Some(kind) = parameter_attr_kind(attr) else {
            continue;
        };
        let name = match kind {
            ParameterAttrKind::PathParam | ParameterAttrKind::QueryParam => {
                parse_optional_string_arg(attr, kind.name())?
                    .unwrap_or_else(|| inferred_name.to_string())
            }
            ParameterAttrKind::Header => parse_required_string_arg(attr, kind.name())?,
        };
        validate_parameter_name(attr, &name, kind.name())?;
        if matches!(kind, ParameterAttrKind::Header) {
            validate_header_name(attr, &name)?;
        }
        bindings.push(ParameterAttr { kind, name });
        remove_indices.push(index);
    }

    for index in remove_indices.into_iter().rev() {
        attrs.remove(index);
    }

    Ok(bindings)
}

impl ParameterAttrKind {
    fn name(&self) -> &'static str {
        match self {
            Self::PathParam => "path",
            Self::QueryParam => "query",
            Self::Header => "header",
        }
    }
}

fn parameter_attr_kind(attr: &Attribute) -> Option<ParameterAttrKind> {
    if is_catnap_attr(attr, "path") {
        Some(ParameterAttrKind::PathParam)
    } else if is_catnap_attr(attr, "query") {
        Some(ParameterAttrKind::QueryParam)
    } else if is_catnap_attr(attr, "header") {
        Some(ParameterAttrKind::Header)
    } else {
        None
    }
}

fn matching_attr_indices(attrs: &[Attribute], name: &str) -> Vec<usize> {
    attrs
        .iter()
        .enumerate()
        .filter_map(|(index, attr)| is_catnap_attr(attr, name).then_some(index))
        .collect()
}

fn is_catnap_attr(attr: &Attribute, name: &str) -> bool {
    let path = attr.path();
    if path.is_ident(name) {
        return true;
    }

    path.leading_colon.is_none()
        && path.segments.len() == 2
        && path.segments[0].ident == "catnap"
        && path.segments[1].ident == name
}

fn parse_optional_string_arg(
    attr: &Attribute,
    description: &'static str,
) -> syn::Result<Option<String>> {
    attr.parse_args_with(|input: syn::parse::ParseStream<'_>| {
        if input.is_empty() {
            return Ok(None);
        }

        let value: LitStr = input.parse()?;
        if !input.is_empty() {
            return Err(input.error(format!("{description} accept at most one string literal")));
        }
        Ok(Some(value.value()))
    })
}

fn parse_required_string_arg(attr: &Attribute, name: &str) -> syn::Result<String> {
    attr.parse_args_with(|input: syn::parse::ParseStream<'_>| {
        if input.is_empty() {
            return Err(input.error(format!("#[{name}] requires a string literal argument")));
        }

        let value: LitStr = input.parse()?;
        if !input.is_empty() {
            return Err(input.error(format!("#[{name}] accepts exactly one string literal")));
        }
        Ok(value.value())
    })
}

fn validate_parameter_name(attr: &Attribute, value: &str, name: &str) -> syn::Result<()> {
    if value.is_empty() {
        return Err(syn::Error::new_spanned(
            attr,
            format!("#[{name}] parameter names must not be empty"),
        ));
    }

    Ok(())
}

fn validate_header_name(attr: &Attribute, value: &str) -> syn::Result<()> {
    if !value.bytes().all(is_header_name_byte) {
        return Err(syn::Error::new_spanned(
            attr,
            format!("#[header] value `{value}` is not a valid HTTP header name"),
        ));
    }

    Ok(())
}

fn is_header_name_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric()
        || matches!(
            byte,
            b'!' | b'#'
                | b'$'
                | b'%'
                | b'&'
                | b'\''
                | b'*'
                | b'+'
                | b'-'
                | b'.'
                | b'^'
                | b'_'
                | b'`'
                | b'|'
                | b'~'
        )
}

fn validate_path_template_lit(lit: &LitStr) -> syn::Result<String> {
    let value = lit.value();
    validate_path_template(&value, lit.span())?;
    Ok(value)
}

fn validate_path_template(value: &str, span: proc_macro2::Span) -> syn::Result<()> {
    path_placeholders(value)
        .map(|_| ())
        .map_err(|message| syn::Error::new(span, format!("invalid REST client path: {message}")))
}

fn validate_media_type_lit(lit: &LitStr) -> syn::Result<MediaType> {
    validate_media_type(&lit.value(), lit.span())
}

fn validate_media_type(value: &str, span: proc_macro2::Span) -> syn::Result<MediaType> {
    let (kind, subtype) = value
        .split_once('/')
        .ok_or_else(|| syn::Error::new(span, "media types must use `type/subtype` syntax"))?;

    if kind.is_empty()
        || subtype.is_empty()
        || value.chars().any(char::is_whitespace)
        || kind.contains(';')
    {
        return Err(syn::Error::new(
            span,
            "media types must be non-empty `type/subtype` values without whitespace",
        ));
    }

    Ok(MediaType::parse(value))
}

fn validate_paths(full_path: &str, path_names: &[String], method: &TraitItemFn) -> syn::Result<()> {
    let placeholders = path_placeholders(full_path).map_err(|message| {
        syn::Error::new_spanned(
            &method.sig.ident,
            format!("invalid REST client path: {message}"),
        )
    })?;

    if has_duplicate(&placeholders) {
        return Err(syn::Error::new_spanned(
            &method.sig.ident,
            "REST client path placeholders must be unique",
        ));
    }

    for placeholder in &placeholders {
        if !path_names.iter().any(|name| name == placeholder) {
            return Err(syn::Error::new_spanned(
                &method.sig.ident,
                format!("missing #[path(\"{placeholder}\")] argument for path placeholder"),
            ));
        }
    }

    for name in path_names {
        if !placeholders.iter().any(|placeholder| placeholder == name) {
            return Err(syn::Error::new_spanned(
                &method.sig.ident,
                format!("#[path(\"{name}\")] does not match a path placeholder"),
            ));
        }
    }

    Ok(())
}

fn path_placeholders(path: &str) -> Result<Vec<String>, String> {
    let mut placeholders = Vec::new();
    let mut chars = path.char_indices().peekable();

    while let Some((index, ch)) = chars.next() {
        match ch {
            '{' => {
                let start = index + ch.len_utf8();
                let mut end = None;
                for (next_index, next_ch) in chars.by_ref() {
                    match next_ch {
                        '{' => return Err("nested `{` in path placeholder".to_owned()),
                        '}' => {
                            end = Some(next_index);
                            break;
                        }
                        _ => {}
                    }
                }

                let Some(end) = end else {
                    return Err("unclosed `{` in path placeholder".to_owned());
                };
                let placeholder = &path[start..end];
                validate_path_placeholder(placeholder)?;
                placeholders.push(placeholder.to_owned());
            }
            '}' => return Err("unmatched `}` in path".to_owned()),
            _ => {}
        }
    }

    Ok(placeholders)
}

fn validate_path_placeholder(placeholder: &str) -> Result<(), String> {
    if placeholder.is_empty() {
        return Err("path placeholders must not be empty".to_owned());
    }
    if placeholder.chars().any(char::is_whitespace) {
        return Err(format!(
            "path placeholder `{placeholder}` must not contain whitespace"
        ));
    }
    if placeholder.contains('/') {
        return Err(format!(
            "path placeholder `{placeholder}` must describe one path segment"
        ));
    }

    Ok(())
}

fn has_duplicate(values: &[String]) -> bool {
    values
        .iter()
        .enumerate()
        .any(|(index, value)| values[index + 1..].iter().any(|other| other == value))
}

fn pat_ident(pat: &Pat) -> Option<Ident> {
    match pat {
        Pat::Ident(pat_ident) if pat_ident.by_ref.is_none() && pat_ident.mutability.is_none() => {
            Some(pat_ident.ident.clone())
        }
        _ => None,
    }
}

fn body_for(
    catnap: &TokenStream2,
    request_var: &Ident,
    body_arg: Option<Ident>,
    consumes: &MediaType,
) -> syn::Result<TokenStream2> {
    let Some(arg) = body_arg else {
        return Ok(quote! {});
    };

    let media_type = consumes.as_str();
    match BodyMode::for_media_type(consumes) {
        BodyMode::Json => Ok(quote! {
            #request_var = #request_var.content_type(#media_type).json(#arg)?;
        }),
        BodyMode::Xml => Ok(quote! {
            #request_var = #request_var.content_type(#media_type).xml(#arg)?;
        }),
        BodyMode::Text => Ok(quote! {
            #request_var = #request_var.content_type(#media_type).text(#arg);
        }),
        BodyMode::Unsupported => Ok(quote! {
            return Err(#catnap::Error::UnsupportedMediaType {
                operation: #catnap::MediaOperation::RequestSerialization,
                media_type: #media_type,
            });
        }),
    }
}

enum BodyMode {
    Json,
    Xml,
    Text,
    Unsupported,
}

impl BodyMode {
    fn for_media_type(media_type: &MediaType) -> Self {
        match media_type {
            MediaType::Json => Self::Json,
            MediaType::Xml | MediaType::TextXml => Self::Xml,
            MediaType::TextPlain => Self::Text,
            MediaType::Other(_) => Self::Unsupported,
        }
    }
}

fn sender_for(
    catnap: &TokenStream2,
    request_var: &Ident,
    output: &ReturnType,
    produces: &MediaType,
) -> syn::Result<TokenStream2> {
    let ReturnType::Type(_, ty) = output else {
        return Err(syn::Error::new_spanned(
            output,
            "REST client methods must return catnap::Result<T>",
        ));
    };
    let inner = result_inner(ty)?;
    let media_type = produces.as_str();
    match ResponseMode::for_return_type(inner, produces) {
        ResponseMode::Raw => Ok(quote! { #request_var.send().await }),
        ResponseMode::Empty => Ok(quote! { #request_var.send_empty().await }),
        ResponseMode::Text => Ok(quote! { #request_var.send_text().await }),
        ResponseMode::Json(inner) => Ok(quote! { #request_var.send_json::<#inner>().await }),
        ResponseMode::Xml(inner) => Ok(quote! { #request_var.send_xml::<#inner>().await }),
        ResponseMode::Unsupported => Ok(quote! {
            Err(#catnap::Error::UnsupportedMediaType {
                operation: #catnap::MediaOperation::ResponseDeserialization,
                media_type: #media_type,
            })
        }),
    }
}

enum ResponseMode<'a> {
    Raw,
    Empty,
    Text,
    Json(&'a Type),
    Xml(&'a Type),
    Unsupported,
}

impl<'a> ResponseMode<'a> {
    fn for_return_type(inner: &'a Type, media_type: &MediaType) -> Self {
        if is_response(inner) {
            Self::Raw
        } else if is_unit(inner) {
            Self::Empty
        } else if is_string(inner) && matches!(media_type, MediaType::TextPlain) {
            Self::Text
        } else if matches!(media_type, MediaType::Json) {
            Self::Json(inner)
        } else if media_type.is_xml() {
            Self::Xml(inner)
        } else {
            Self::Unsupported
        }
    }
}

fn result_inner(ty: &Type) -> syn::Result<&Type> {
    let Type::Path(type_path) = ty else {
        return Err(syn::Error::new_spanned(
            ty,
            "REST client methods must return catnap::Result<T>",
        ));
    };
    let Some(segment) = type_path.path.segments.last() else {
        return Err(syn::Error::new_spanned(
            ty,
            "REST client methods must return catnap::Result<T>",
        ));
    };
    if segment.ident != "Result" {
        return Err(syn::Error::new_spanned(
            ty,
            "REST client methods must return catnap::Result<T>",
        ));
    }
    let syn::PathArguments::AngleBracketed(args) = &segment.arguments else {
        return Err(syn::Error::new_spanned(
            ty,
            "REST client methods must return catnap::Result<T>",
        ));
    };

    let mut type_args = args.args.iter().filter_map(|arg| match arg {
        syn::GenericArgument::Type(ty) => Some(ty),
        _ => None,
    });
    let Some(inner) = type_args.next() else {
        return Err(syn::Error::new_spanned(
            ty,
            "REST client methods must return catnap::Result<T> with one type parameter",
        ));
    };
    if type_args.next().is_some() || args.args.len() != 1 {
        return Err(syn::Error::new_spanned(
            ty,
            "REST client methods must return catnap::Result<T> with one type parameter",
        ));
    }

    Ok(inner)
}

fn is_response(ty: &Type) -> bool {
    matches!(ty, Type::Path(type_path) if type_path.path.segments.last().is_some_and(|segment| segment.ident == "Response"))
}

fn is_unit(ty: &Type) -> bool {
    matches!(ty, Type::Tuple(tuple) if tuple.elems.is_empty())
}

fn is_string(ty: &Type) -> bool {
    matches!(ty, Type::Path(type_path) if type_path.path.segments.last().is_some_and(|segment| segment.ident == "String"))
}

fn is_query_collection(ty: &Type) -> bool {
    match ty {
        Type::Array(_) => true,
        Type::Reference(reference) => is_query_collection_reference_target(&reference.elem),
        Type::Path(type_path) => type_path
            .path
            .segments
            .last()
            .is_some_and(|segment| is_query_collection_ident(&segment.ident)),
        _ => false,
    }
}

fn is_query_collection_reference_target(ty: &Type) -> bool {
    match ty {
        Type::Slice(_) => true,
        Type::Array(_) => true,
        Type::Path(type_path) => type_path
            .path
            .segments
            .last()
            .is_some_and(|segment| is_query_collection_ident(&segment.ident)),
        _ => false,
    }
}

fn is_query_collection_ident(ident: &Ident) -> bool {
    matches!(
        ident.to_string().as_str(),
        "Vec" | "VecDeque" | "LinkedList" | "HashSet" | "BTreeSet" | "BinaryHeap"
    )
}

fn join_paths(base: &str, method: &str) -> String {
    match (base.trim_end_matches('/'), method.trim_start_matches('/')) {
        ("", "") => "/".to_owned(),
        ("", method) => format!("/{method}"),
        (base, "") => base.to_owned(),
        (base, method) => format!("{base}/{method}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_path_placeholders() {
        assert_eq!(
            path_placeholders("/users/{user_id}/posts/{post_id}").expect("valid placeholders"),
            ["user_id".to_owned(), "post_id".to_owned()],
        );
    }

    #[test]
    fn rejects_malformed_path_placeholders() {
        assert_eq!(
            path_placeholders("/users/{user_id").expect_err("path should be invalid"),
            "unclosed `{` in path placeholder",
        );
        assert_eq!(
            path_placeholders("/users/{}").expect_err("path should be invalid"),
            "path placeholders must not be empty",
        );
        assert_eq!(
            path_placeholders("/users/{user id}").expect_err("path should be invalid"),
            "path placeholder `user id` must not contain whitespace",
        );
    }

    #[test]
    fn recognizes_qualified_catnap_attributes() {
        let attr: Attribute = syn::parse_quote!(#[catnap::produces("text/plain")]);

        assert!(is_catnap_attr(&attr, "produces"));
        assert!(!is_catnap_attr(&attr, "consumes"));
    }

    #[test]
    fn recognizes_common_query_collections() {
        assert!(is_query_collection(&syn::parse_quote!(Vec<String>)));
        assert!(is_query_collection(&syn::parse_quote!(&Vec<String>)));
        assert!(is_query_collection(&syn::parse_quote!(&[String])));
        assert!(is_query_collection(&syn::parse_quote!([String; 2])));
        assert!(is_query_collection(&syn::parse_quote!(
            std::collections::HashSet<String>
        )));

        assert!(!is_query_collection(&syn::parse_quote!(String)));
        assert!(!is_query_collection(&syn::parse_quote!(&str)));
        assert!(!is_query_collection(&syn::parse_quote!(u32)));
    }
}
