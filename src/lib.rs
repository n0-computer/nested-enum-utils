use std::collections::BTreeSet;

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::{quote, ToTokens};
use syn::{
    parse::{Parse, ParseStream},
    parse_macro_input,
    punctuated::Punctuated,
    Data, DeriveInput, Fields, Ident, Token, Type, Variant,
};

fn extract_enum_variants(input: &DeriveInput) -> syn::Result<Vec<(&syn::Ident, &syn::Type)>> {
    let mut distinct_types = BTreeSet::new();
    let Data::Enum(data_enum) = &input.data else {
        return Err(syn::Error::new_spanned(
            input,
            "EnumConversions can only be used with enums",
        ));
    };
    data_enum.variants.iter().map(|variant: &Variant| {
        let variant_name = &variant.ident;
        match &variant.fields {
            Fields::Unnamed(fields) if fields.unnamed.len() == 1 => {
                let field_type = &fields.unnamed.first().unwrap().ty;
                if !distinct_types.insert(field_type.to_token_stream().to_string()) {
                    return Err(syn::Error::new_spanned(
                        field_type,
                        "EnumConversions only works with enums that have unnamed single fields of distinct types"
                    ));
                }
                Ok((variant_name, field_type))
            },
            _ => Err(syn::Error::new_spanned(
                variant,
                "EnumConversions only works with enums that have unnamed single fields"
            ))
        }
    }).collect()
}

fn generate_enum_self_conversions(enum_name: &Ident, variants: &[(&Ident, &Type)]) -> TokenStream2 {
    let mut conversions = TokenStream2::new();

    for (variant_name, field_type) in variants {
        // Generate From<FieldType> for Enum
        let from_impl = quote! {
            impl From<#field_type> for #enum_name {
                fn from(value: #field_type) -> Self {
                    #enum_name::#variant_name(value)
                }
            }
        };
        conversions.extend(from_impl);

        // Generate TryFrom<Enum> for FieldType
        //
        // This is a self conversion, so it case it does not work we want to return the original value
        let try_from_impl = quote! {
            impl TryFrom<#enum_name> for #field_type {
                type Error = #enum_name;

                fn try_from(value: #enum_name) -> Result<Self, Self::Error> {
                    match value {
                        #enum_name::#variant_name(inner) => Ok(inner),
                        x => Err(x),
                    }
                }
            }
        };
        conversions.extend(try_from_impl);

        // Generate TryFrom<Enum> for &FieldType
        let try_from_ref_impl = quote! {
            impl<'a> TryFrom<&'a #enum_name> for &'a #field_type {
                type Error = &'a #enum_name;

                fn try_from(value: &'a #enum_name) -> Result<Self, Self::Error> {
                    match value {
                        #enum_name::#variant_name(inner) => Ok(inner),
                        _ => Err(value),
                    }
                }
            }
        };
        conversions.extend(try_from_ref_impl);
    }

    conversions
}

fn generate_enum_target_conversions(
    enum_name: &Ident,
    target_type: &Type,
    variants: &[(&Ident, &Type)],
) -> TokenStream2 {
    let mut conversions = TokenStream2::new();

    for (variant_name, field_type) in variants {
        // Generate From<FieldType> for TargetType
        let from_impl = quote! {
            impl From<#field_type> for #target_type {
                fn from(value: #field_type) -> Self {
                    let enum_value = #enum_name::#variant_name(value);
                    Self::from(enum_value)
                }
            }
        };
        conversions.extend(from_impl);

        // Generate TryFrom<TargetType> for FieldType
        //
        // This is a self conversion, so it case it does not work we want to return the original value
        let try_from_impl = quote! {
            impl TryFrom<#target_type> for #field_type {
                type Error = #target_type;

                fn try_from(value: #target_type) -> Result<Self, Self::Error> {
                    match #enum_name::try_from(value) {
                        Ok(#enum_name::#variant_name(inner)) => Ok(inner),
                        Ok(x) => Err(#target_type::from(x)),
                        Err(x) => Err(x),
                    }
                }
            }
        };
        conversions.extend(try_from_impl);

        // Generate TryFrom<&TargetType> for &FieldType
        //
        // This is a self conversion, so it case it does not work we want to return the original value
        let try_from_ref_impl = quote! {
            impl<'a> TryFrom<&'a #target_type> for &'a #field_type {
                type Error = &'a #target_type;

                fn try_from(value: &'a #target_type) -> Result<Self, Self::Error> {
                    match <&'a #enum_name>::try_from(value) {
                        Ok(#enum_name::#variant_name(inner)) => Ok(inner),
                        Ok(_) => Err(value),
                        Err(_) => Err(value),
                    }
                }
            }
        };
        conversions.extend(try_from_ref_impl);
    }

    conversions
}

struct EnumConversionsArgs {
    target_types: Punctuated<Type, Token![,]>,
}

impl Parse for EnumConversionsArgs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        Ok(EnumConversionsArgs {
            target_types: Punctuated::parse_terminated(input)?,
        })
    }
}

/// Derive macro that generates conversions between an enum and its variants and other types.
///
/// The macro can be used as follows:
///
/// ```rust
/// use nested_enum_utils::enum_conversions;
///
/// #[enum_conversions()]
/// enum MyEnum {
///     Variant1(u32),
///     Variant2(String),
/// }
/// ```
///
/// This will create From instances from each variant type to the enum and TryFrom instances from the enum to each variant type.
///
/// The macro also accepts a list of target types to generate conversions to:
///
/// ```rust
/// use nested_enum_utils::enum_conversions;
///
/// #[enum_conversions(Outer)]
/// enum Inner {
///     Variant1(u32),
///     Variant2(String),
/// }
///
/// #[enum_conversions()]
/// enum Outer {
///     Inner1(Inner),
///     // other variants
/// }
/// ```
///
/// This will, in addition, generate From instances from each variant type to the outer enum and TryFrom instances from the outer enum to each variant type.
/// The conversion to the outer enum relies on conversions between the inner enum and the outer enum, which is provided by the
/// enum_conversions attribute on the Outer enum.
///
/// Limitations:
///
/// - enums must have unnamed single fields
/// - field types must be distinct
#[proc_macro_attribute]
pub fn enum_conversions(attr: TokenStream, item: TokenStream) -> TokenStream {
    let args = parse_macro_input!(attr as EnumConversionsArgs);
    let input = parse_macro_input!(item as DeriveInput);

    let enum_name = &input.ident;

    let variants = match extract_enum_variants(&input) {
        Ok(v) => v,
        Err(e) => return e.to_compile_error().into(),
    };

    let mut all_conversions = TokenStream2::new();

    // Generate self-conversions
    all_conversions.extend(generate_enum_self_conversions(enum_name, &variants));

    // Generate conversions for each target type
    for target_type in args.target_types {
        all_conversions.extend(generate_enum_target_conversions(
            enum_name,
            &target_type,
            &variants,
        ));
    }

    let expanded = quote! {
        #input
        #all_conversions
    };
    TokenStream::from(expanded)
}
