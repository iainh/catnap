use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote};
use syn::{
    Attribute, FnArg, Ident, ItemTrait, LitStr, Pat, ReturnType, TraitItem, TraitItemFn, Type,
    parse_macro_input,
};

#[proc_macro_attribute]
pub fn rest_client(attr: TokenStream, item: TokenStream) -> TokenStream {
    let args = parse_macro_input!(attr as RestClientArgs);
    let mut trait_item = parse_macro_input!(item as ItemTrait);
    let expanded = expand_rest_client(args, &mut trait_item);
    TokenStream::from(expanded)
}

#[proc_macro_attribute]
pub fn get(_: TokenStream, item: TokenStream) -> TokenStream {
    item
}

#[proc_macro_attribute]
pub fn post(_: TokenStream, item: TokenStream) -> TokenStream {
    item
}

#[proc_macro_attribute]
pub fn put(_: TokenStream, item: TokenStream) -> TokenStream {
    item
}

#[proc_macro_attribute]
pub fn patch(_: TokenStream, item: TokenStream) -> TokenStream {
    item
}

#[proc_macro_attribute]
pub fn delete(_: TokenStream, item: TokenStream) -> TokenStream {
    item
}

#[proc_macro_attribute]
pub fn options(_: TokenStream, item: TokenStream) -> TokenStream {
    item
}

#[proc_macro_attribute]
pub fn head(_: TokenStream, item: TokenStream) -> TokenStream {
    item
}

#[proc_macro_attribute]
pub fn path(_: TokenStream, item: TokenStream) -> TokenStream {
    item
}

#[proc_macro_attribute]
pub fn query(_: TokenStream, item: TokenStream) -> TokenStream {
    item
}

#[proc_macro_attribute]
pub fn header(_: TokenStream, item: TokenStream) -> TokenStream {
    item
}

struct RestClientArgs {
    path: String,
}

impl syn::parse::Parse for RestClientArgs {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        if input.is_empty() {
            return Ok(Self {
                path: String::new(),
            });
        }
        let key: Ident = input.parse()?;
        if key != "path" {
            return Err(syn::Error::new(key.span(), "expected `path = \"...\"`"));
        }
        input.parse::<syn::Token![=]>()?;
        let path: LitStr = input.parse()?;
        Ok(Self { path: path.value() })
    }
}

fn expand_rest_client(args: RestClientArgs, trait_item: &mut ItemTrait) -> TokenStream2 {
    trait_item
        .attrs
        .push(syn::parse_quote!(#[allow(async_fn_in_trait)]));

    let trait_ident = trait_item.ident.clone();
    let client_ident = format_ident!("{trait_ident}Client");
    let vis = trait_item.vis.clone();
    let base_path = args.path;

    let impl_methods = trait_item
        .items
        .iter_mut()
        .filter_map(|item| match item {
            TraitItem::Fn(method) => Some(expand_method(&base_path, method)),
            _ => None,
        })
        .collect::<Vec<_>>();

    quote! {
        #trait_item

        #[derive(Clone, Debug)]
        #vis struct #client_ident {
            inner: ::catnap::RestClient,
        }

        impl #client_ident {
            pub fn builder() -> ::catnap::RestClientBuilder<Self> {
                ::catnap::RestClientBuilder::new()
            }
        }

        impl ::catnap::BuildFromConfig for #client_ident {
            fn build_from_config(config: ::catnap::RestClientConfig) -> ::catnap::Result<Self> {
                Ok(Self {
                    inner: ::catnap::RestClient::from_config(config)?,
                })
            }
        }

        impl #trait_ident for #client_ident {
            #(#impl_methods)*
        }
    }
}

fn expand_method(base_path: &str, method: &mut TraitItemFn) -> TokenStream2 {
    let (verb, method_path) =
        take_http_attr(&mut method.attrs).unwrap_or_else(|| ("GET".to_owned(), String::new()));
    let full_path = join_paths(base_path, &method_path);

    let mut path_replacements = Vec::new();
    let mut query_params = Vec::new();
    let mut header_params = Vec::new();
    let mut body_arg = None;

    for input in &mut method.sig.inputs {
        let FnArg::Typed(pat_type) = input else {
            continue;
        };
        let arg_ident = pat_ident(&pat_type.pat);
        let Some(arg_ident) = arg_ident else {
            continue;
        };
        if let Some(name) = take_named_attr(&mut pat_type.attrs, "path") {
            path_replacements.push(quote! {
                path = path.replace(
                    concat!("{", #name, "}"),
                    &::std::string::ToString::to_string(#arg_ident),
                );
            });
        } else if let Some(name) = take_named_attr(&mut pat_type.attrs, "query") {
            query_params.push(quote! {
                request = request.query_param(#name, #arg_ident);
            });
        } else if let Some(name) = take_named_attr(&mut pat_type.attrs, "header") {
            header_params.push(quote! {
                request = request.header(#name, #arg_ident);
            });
        } else if body_arg.is_none() {
            body_arg = Some(arg_ident);
        }
    }

    let sig = method.sig.clone();
    let output = &sig.output;
    let body = body_arg.map(|arg| quote! { request = request.json(#arg); });
    let sender = sender_for(output);
    let verb_ident = format_ident!("{}", verb);

    quote! {
        #sig {
            let mut path = #full_path.to_owned();
            #(#path_replacements)*
            let mut request = self.inner.request(::catnap::http::Method::#verb_ident, &path);
            #(#query_params)*
            #(#header_params)*
            #body
            #sender
        }
    }
}

fn take_http_attr(attrs: &mut Vec<Attribute>) -> Option<(String, String)> {
    for verb in ["get", "post", "put", "patch", "delete", "options", "head"] {
        if let Some((index, attr)) = attrs
            .iter()
            .enumerate()
            .find(|(_, attr)| attr.path().is_ident(verb))
        {
            let path = attr
                .parse_args::<LitStr>()
                .map(|lit| lit.value())
                .unwrap_or_default();
            attrs.remove(index);
            return Some((verb.to_uppercase(), path));
        }
    }
    None
}

fn take_named_attr(attrs: &mut Vec<Attribute>, name: &str) -> Option<String> {
    let (index, attr) = attrs
        .iter()
        .enumerate()
        .find(|(_, attr)| attr.path().is_ident(name))?;
    let value = attr.parse_args::<LitStr>().ok()?.value();
    attrs.remove(index);
    Some(value)
}

fn pat_ident(pat: &Pat) -> Option<Ident> {
    match pat {
        Pat::Ident(pat_ident) => Some(pat_ident.ident.clone()),
        _ => None,
    }
}

fn sender_for(output: &ReturnType) -> TokenStream2 {
    let ReturnType::Type(_, ty) = output else {
        return quote! { request.send_empty().await };
    };
    let Some(inner) = result_inner(ty) else {
        return quote! { request.send_empty().await };
    };
    if is_response(inner) {
        quote! { request.send().await }
    } else if is_unit(inner) {
        quote! { request.send_empty().await }
    } else {
        quote! { request.send_json::<#inner>().await }
    }
}

fn result_inner(ty: &Type) -> Option<&Type> {
    let Type::Path(type_path) = ty else {
        return None;
    };
    let segment = type_path.path.segments.last()?;
    if segment.ident != "Result" {
        return None;
    }
    let syn::PathArguments::AngleBracketed(args) = &segment.arguments else {
        return None;
    };
    args.args.iter().find_map(|arg| match arg {
        syn::GenericArgument::Type(ty) => Some(ty),
        _ => None,
    })
}

fn is_response(ty: &Type) -> bool {
    matches!(ty, Type::Path(type_path) if type_path.path.segments.last().is_some_and(|segment| segment.ident == "Response"))
}

fn is_unit(ty: &Type) -> bool {
    matches!(ty, Type::Tuple(tuple) if tuple.elems.is_empty())
}

fn join_paths(base: &str, method: &str) -> String {
    match (base.trim_end_matches('/'), method.trim_start_matches('/')) {
        ("", "") => "/".to_owned(),
        ("", method) => format!("/{method}"),
        (base, "") => base.to_owned(),
        (base, method) => format!("{base}/{method}"),
    }
}
