use proc_macro::TokenStream;
use proc_macro_crate::{FoundCrate, crate_name};
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

#[proc_macro_attribute]
pub fn consumes(_: TokenStream, item: TokenStream) -> TokenStream {
    item
}

#[proc_macro_attribute]
pub fn produces(_: TokenStream, item: TokenStream) -> TokenStream {
    item
}

struct RestClientArgs {
    path: String,
    consumes: String,
    produces: String,
}

impl syn::parse::Parse for RestClientArgs {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        let mut args = Self {
            path: String::new(),
            consumes: APPLICATION_JSON.to_owned(),
            produces: APPLICATION_JSON.to_owned(),
        };
        if input.is_empty() {
            return Ok(args);
        }

        while !input.is_empty() {
            let key: Ident = input.parse()?;
            input.parse::<syn::Token![=]>()?;
            let value: LitStr = input.parse()?;
            match key.to_string().as_str() {
                "path" => args.path = value.value(),
                "consumes" => args.consumes = validate_media_type(&value)?,
                "produces" => args.produces = validate_media_type(&value)?,
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

const APPLICATION_JSON: &str = "application/json";
const APPLICATION_XML: &str = "application/xml";
const TEXT_PLAIN: &str = "text/plain";
const TEXT_XML: &str = "text/xml";

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
    match crate_name("catnap") {
        Ok(FoundCrate::Itself) => quote!(::catnap),
        Ok(FoundCrate::Name(name)) => {
            let ident = Ident::new(&name, proc_macro2::Span::call_site());
            quote!(::#ident)
        }
        Err(_) => quote!(::catnap),
    }
}

fn expand_method(
    catnap: &TokenStream2,
    trait_config: &ResourceConfig,
    method: &mut TraitItemFn,
) -> syn::Result<TokenStream2> {
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
    let mut path_param_names = Vec::new();

    for input in &mut method.sig.inputs {
        let FnArg::Typed(pat_type) = input else {
            continue;
        };
        let arg_ident = pat_ident(&pat_type.pat);
        let Some(arg_ident) = arg_ident else {
            continue;
        };
        if let Some(name) = take_named_attr(&mut pat_type.attrs, "path") {
            path_param_names.push(name.clone());
            path_replacements.push(quote! {
                path = path.replace(
                    concat!("{", #name, "}"),
                    &#catnap::__private::encode_path_segment(#arg_ident),
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
        } else if body_arg.replace(arg_ident).is_some() {
            return Err(syn::Error::new_spanned(
                pat_type,
                "REST client methods may have at most one unannotated body argument",
            ));
        }
    }

    validate_path_params(&full_path, &path_param_names, method)?;

    let sig = method.sig.clone();
    let output = &sig.output;
    let body = body_for(catnap, body_arg, &consumes)?;
    let sender = sender_for(catnap, output, &produces)?;
    let verb_ident = format_ident!("{}", verb);

    Ok(quote! {
        #sig {
            let mut path = #full_path.to_owned();
            #(#path_replacements)*
            let mut request = self.inner.request(#catnap::http::Method::#verb_ident, &path)?;
            request = request.accept(#produces);
            #(#query_params)*
            #(#header_params)*
            #body
            #sender
        }
    })
}

fn take_http_attr(attrs: &mut Vec<Attribute>) -> syn::Result<Option<(String, String)>> {
    let mut found = None;
    let mut index = 0;
    while index < attrs.len() {
        let attr = &attrs[index];
        let Some(verb) = ["get", "post", "put", "patch", "delete", "options", "head"]
            .into_iter()
            .find(|verb| attr.path().is_ident(verb))
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

        let path = attr
            .parse_args::<LitStr>()
            .map(|lit| lit.value())
            .unwrap_or_default();
        attrs.remove(index);
        found = Some((verb.to_uppercase(), path));
    }
    Ok(found)
}

#[derive(Debug)]
struct ResourceConfig {
    path: String,
    consumes: String,
    produces: String,
}

fn take_media_type_attr(attrs: &mut Vec<Attribute>, name: &str) -> syn::Result<Option<String>> {
    let Some((index, attr)) = attrs
        .iter()
        .enumerate()
        .find(|(_, attr)| attr.path().is_ident(name))
    else {
        return Ok(None);
    };

    let value = validate_media_type(&attr.parse_args::<LitStr>()?)?;
    attrs.remove(index);
    Ok(Some(value))
}

fn validate_media_type(lit: &LitStr) -> syn::Result<String> {
    let value = lit.value();
    let (kind, subtype) = value.split_once('/').ok_or_else(|| {
        syn::Error::new_spanned(lit, "media types must use `type/subtype` syntax")
    })?;

    if kind.is_empty()
        || subtype.is_empty()
        || value.chars().any(char::is_whitespace)
        || kind.contains(';')
    {
        return Err(syn::Error::new_spanned(
            lit,
            "media types must be non-empty `type/subtype` values without whitespace",
        ));
    }

    Ok(value)
}

fn validate_path_params(
    full_path: &str,
    path_param_names: &[String],
    method: &TraitItemFn,
) -> syn::Result<()> {
    let placeholders = path_placeholders(full_path);
    for placeholder in &placeholders {
        if !path_param_names.iter().any(|name| name == placeholder) {
            return Err(syn::Error::new_spanned(
                &method.sig.ident,
                format!("missing #[path(\"{placeholder}\")] argument for path placeholder"),
            ));
        }
    }

    for name in path_param_names {
        if !placeholders.iter().any(|placeholder| placeholder == name) {
            return Err(syn::Error::new_spanned(
                &method.sig.ident,
                format!("#[path(\"{name}\")] does not match a path placeholder"),
            ));
        }
    }

    Ok(())
}

fn path_placeholders(path: &str) -> Vec<String> {
    let mut placeholders = Vec::new();
    let mut rest = path;
    while let Some(start) = rest.find('{') {
        rest = &rest[start + 1..];
        let Some(end) = rest.find('}') else {
            break;
        };
        placeholders.push(rest[..end].to_owned());
        rest = &rest[end + 1..];
    }
    placeholders
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

fn body_for(
    catnap: &TokenStream2,
    body_arg: Option<Ident>,
    consumes: &str,
) -> syn::Result<TokenStream2> {
    let Some(arg) = body_arg else {
        return Ok(quote! {});
    };

    match consumes {
        APPLICATION_JSON => Ok(quote! {
            request = request.content_type(#consumes).json(#arg);
        }),
        APPLICATION_XML | TEXT_XML => Ok(quote! {
            request = request.content_type(#consumes).xml(#arg)?;
        }),
        TEXT_PLAIN => Ok(quote! {
            request = request.content_type(#consumes).text(#arg);
        }),
        _ => Ok(quote! {
            return Err(#catnap::Error::UnsupportedMediaType {
                operation: "request body serialization",
                media_type: #consumes,
            });
        }),
    }
}

fn sender_for(
    catnap: &TokenStream2,
    output: &ReturnType,
    produces: &str,
) -> syn::Result<TokenStream2> {
    let ReturnType::Type(_, ty) = output else {
        return Err(syn::Error::new_spanned(
            output,
            "REST client methods must return catnap::Result<T>",
        ));
    };
    let Some(inner) = result_inner(ty) else {
        return Err(syn::Error::new_spanned(
            ty,
            "REST client methods must return catnap::Result<T>",
        ));
    };
    if is_response(inner) {
        Ok(quote! { request.send().await })
    } else if is_unit(inner) {
        Ok(quote! { request.send_empty().await })
    } else if is_string(inner) && produces == TEXT_PLAIN {
        Ok(quote! { request.send_text().await })
    } else if produces == APPLICATION_JSON {
        Ok(quote! { request.send_json::<#inner>().await })
    } else if matches!(produces, APPLICATION_XML | TEXT_XML) {
        Ok(quote! { request.send_xml::<#inner>().await })
    } else {
        Ok(quote! {
            Err(#catnap::Error::UnsupportedMediaType {
                operation: "response deserialization",
                media_type: #produces,
            })
        })
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

fn is_string(ty: &Type) -> bool {
    matches!(ty, Type::Path(type_path) if type_path.path.segments.last().is_some_and(|segment| segment.ident == "String"))
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
            path_placeholders("/users/{user_id}/posts/{post_id}"),
            ["user_id".to_owned(), "post_id".to_owned()]
        );
    }
}
