// Copyright 2020 Google LLC
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//    https://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use proc_macro2::Span;
use syn::{
    parse::{Parse, ParseStream},
    LitStr, Token,
};
use syn::{Ident, Result as ParseResult};

#[derive(PartialEq, Clone, Debug, Hash)]
pub enum UnsafePolicy {
    AllFunctionsSafe,
    AllFunctionsUnsafe,
}

impl Parse for UnsafePolicy {
    fn parse(input: ParseStream) -> ParseResult<Self> {
        if input.parse::<Option<Token![unsafe]>>()?.is_some() {
            return Ok(UnsafePolicy::AllFunctionsSafe);
        }
        let r = match input.parse::<Option<syn::Ident>>()? {
            Some(id) => {
                if id == "unsafe_ffi" {
                    Ok(UnsafePolicy::AllFunctionsSafe)
                } else {
                    Err(syn::Error::new(id.span(), "expected unsafe_ffi"))
                }
            }
            None => Ok(UnsafePolicy::AllFunctionsUnsafe),
        };
        if !input.is_empty() {
            return Err(syn::Error::new(
                Span::call_site(),
                "unexpected tokens within safety directive",
            ));
        }
        r
    }
}

/// Allowlist configuration.
#[derive(Hash, Debug)]
pub enum Allowlist {
    Unspecified,
    All,
    Specific(Vec<String>),
}

impl Allowlist {
    pub(crate) fn push(&mut self, item: LitStr) -> ParseResult<()> {
        match self {
            Allowlist::Unspecified => {
                *self = Allowlist::Specific(vec![item.value()]);
            }
            Allowlist::All => {
                return Err(syn::Error::new(
                    item.span(),
                    "use either generate!/generate_pod! or generate_all!, not both.",
                ))
            }
            Allowlist::Specific(list) => list.push(item.value()),
        };
        Ok(())
    }

    pub(crate) fn set_all(&mut self, ident: &Ident) -> ParseResult<()> {
        if matches!(self, Allowlist::Specific(..)) {
            return Err(syn::Error::new(
                ident.span(),
                "use either generate!/generate_pod! or generate_all!, not both.",
            ));
        }
        *self = Allowlist::All;
        Ok(())
    }
}

impl Default for Allowlist {
    fn default() -> Self {
        Allowlist::Unspecified
    }
}

#[derive(Hash, Debug)]
pub struct IncludeCppConfig {
    pub inclusions: Vec<String>,
    pub unsafe_policy: UnsafePolicy,
    pub parse_only: bool,
    pod_requests: Vec<String>,
    allowlist: Allowlist,
    blocklist: Vec<String>,
    exclude_utilities: bool,
    mod_name: Option<Ident>,
}

impl Parse for IncludeCppConfig {
    fn parse(input: ParseStream) -> ParseResult<Self> {
        // Takes as inputs:
        // 1. List of headers to include
        // 2. List of #defines to include
        // 3. Allowlist

        let mut inclusions = Vec::new();
        let mut parse_only = false;
        let mut unsafe_policy = UnsafePolicy::AllFunctionsUnsafe;
        let mut allowlist = Allowlist::Unspecified;
        let mut blocklist = Vec::new();
        let mut pod_requests = Vec::new();
        let mut exclude_utilities = false;
        let mut mod_name = None;

        while !input.is_empty() {
            let has_hexathorpe = input.parse::<Option<syn::Token![#]>>()?.is_some();
            let ident: syn::Ident = input.parse()?;
            if has_hexathorpe {
                if ident != "include" {
                    return Err(syn::Error::new(ident.span(), "expected include"));
                }
                let hdr: syn::LitStr = input.parse()?;
                inclusions.push(hdr.value());
            } else {
                input.parse::<Option<syn::Token![!]>>()?;
                if ident == "generate" {
                    let args;
                    syn::parenthesized!(args in input);
                    let generate: syn::LitStr = args.parse()?;
                    allowlist.push(generate)?;
                } else if ident == "generate_pod" {
                    let args;
                    syn::parenthesized!(args in input);
                    let generate_pod: syn::LitStr = args.parse()?;
                    pod_requests.push(generate_pod.value());
                    allowlist.push(generate_pod)?;
                } else if ident == "pod" {
                    let args;
                    syn::parenthesized!(args in input);
                    let pod: syn::LitStr = args.parse()?;
                    pod_requests.push(pod.value());
                } else if ident == "block" {
                    let args;
                    syn::parenthesized!(args in input);
                    let generate: syn::LitStr = args.parse()?;
                    blocklist.push(generate.value());
                } else if ident == "parse_only" {
                    parse_only = true;
                    swallow_parentheses(&input, &ident)?;
                } else if ident == "generate_all" {
                    allowlist.set_all(&ident)?;
                    swallow_parentheses(&input, &ident)?;
                } else if ident == "name" {
                    let args;
                    syn::parenthesized!(args in input);
                    let ident: syn::Ident = args.parse()?;
                    mod_name = Some(ident);
                } else if ident == "exclude_utilities" {
                    exclude_utilities = true;
                    swallow_parentheses(&input, &ident)?;
                } else if ident == "safety" {
                    let args;
                    syn::parenthesized!(args in input);
                    unsafe_policy = args.parse()?;
                } else {
                    return Err(syn::Error::new(
                        ident.span(),
                        "expected generate, generate_pod, nested_type, safety or exclude_utilities",
                    ));
                }
            }
            if input.is_empty() {
                break;
            }
        }

        Ok(IncludeCppConfig {
            inclusions,
            unsafe_policy,
            parse_only,
            pod_requests,
            allowlist,
            blocklist,
            exclude_utilities,
            mod_name,
        })
    }
}

fn swallow_parentheses(input: &ParseStream, latest_ident: &Ident) -> ParseResult<()> {
    let args;
    syn::parenthesized!(args in input);
    if args.is_empty() {
        Ok(())
    } else {
        Err(syn::Error::new(
            latest_ident.span(),
            "expected no arguments to directive",
        ))
    }
}

impl IncludeCppConfig {
    pub fn get_pod_requests(&self) -> &[String] {
        &self.pod_requests
    }

    pub fn get_mod_name(&self) -> Ident {
        self.mod_name
            .as_ref()
            .cloned()
            .unwrap_or_else(|| Ident::new("ffi", Span::call_site()))
    }

    /// Whether to avoid generating the standard helpful utility
    /// functions which we normally include in every mod.
    pub fn exclude_utilities(&self) -> bool {
        self.exclude_utilities
    }

    /// Items which the user has explicitly asked us to generate;
    /// we should raise an error if we weren't able to do so.
    pub fn must_generate_list(&self) -> Box<dyn Iterator<Item = String> + '_> {
        if let Allowlist::Specific(items) = &self.allowlist {
            Box::new(items.iter().chain(self.pod_requests.iter()).cloned())
        } else {
            Box::new(self.pod_requests.iter().cloned())
        }
    }

    /// The allowlist of items to be passed into bindgen, if any.
    pub fn bindgen_allowlist(&self) -> Option<Box<dyn Iterator<Item = String> + '_>> {
        match &self.allowlist {
            Allowlist::All => None,
            Allowlist::Specific(items) => Some(Box::new(
                items
                    .iter()
                    .chain(self.pod_requests.iter())
                    .cloned()
                    .chain(self.active_utilities()),
            )),
            Allowlist::Unspecified => unreachable!(),
        }
    }

    fn active_utilities(&self) -> Vec<String> {
        if self.exclude_utilities {
            Vec::new()
        } else {
            vec![self.get_makestring_name()]
        }
    }

    /// Whether this type is on the allowlist specified by the user.
    ///
    /// A note on the allowlist handling in general. It's used in two places:
    /// 1) As directives to bindgen
    /// 2) After bindgen has generated code, to filter the APIs which
    ///    we pass to cxx.
    /// This second pass may seem redundant. But sometimes bindgen generates
    /// unnecessary stuff.
    pub fn is_on_allowlist(&self, cpp_name: &str) -> bool {
        match self.bindgen_allowlist() {
            None => true,
            Some(mut items) => {
                items.any(|item| item == cpp_name)
                    || self.active_utilities().iter().any(|item| *item == cpp_name)
            }
        }
    }

    pub fn is_on_blocklist(&self, cpp_name: &str) -> bool {
        self.blocklist.contains(&cpp_name.to_string())
    }

    pub fn get_blocklist(&self) -> impl Iterator<Item = &String> {
        self.blocklist.iter()
    }

    pub fn get_makestring_name(&self) -> String {
        format!(
            "autocxx_make_string_{}",
            self.mod_name
                .as_ref()
                .map(|i| i.to_string())
                .unwrap_or_else(|| "default".into())
        )
    }
}

#[cfg(test)]
mod parse_tests {
    use crate::config::UnsafePolicy;
    use syn::parse_quote;
    #[test]
    fn test_safety_unsafe() {
        let us: UnsafePolicy = parse_quote! {
            unsafe
        };
        assert_eq!(us, UnsafePolicy::AllFunctionsSafe)
    }

    #[test]
    fn test_safety_unsafe_ffi() {
        let us: UnsafePolicy = parse_quote! {
            unsafe_ffi
        };
        assert_eq!(us, UnsafePolicy::AllFunctionsSafe)
    }

    #[test]
    fn test_safety_safe() {
        let us: UnsafePolicy = parse_quote! {};
        assert_eq!(us, UnsafePolicy::AllFunctionsUnsafe)
    }
}
