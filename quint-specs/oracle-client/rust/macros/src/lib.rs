//! Proc-macro companion of `quint-oracle`: the `#[quint_oracle::test]`
//! attribute. Use it through the `quint-oracle` re-export — the expansion
//! references `::quint_oracle::` paths, and the contract is documented on
//! that re-export.

use std::str::FromStr;

use proc_macro::{Delimiter, Group, TokenStream, TokenTree};

// Internal by design, and a plain comment rather than a doc comment: rustdoc
// inlines a re-export target's documentation after the `///` block on
// `pub use quint_oracle_macros::test`, so anything documented here would land
// on the client's public page below the user-facing contract that lives there.
//
// Wraps a test function so its run is registered with the oracle before the
// body executes, and its outcome (panic, `Err`, or ok) is reported after:
//
// ```text
// #[test]                    // added unless one is present or the fn is async
// <attrs> <signature> {
//     let __quint_guard = ::quint_oracle::__register_for(module_path!(), "<fn name>");
//     let __quint_ret = (move || <body>)();   // async: (async move <body>).await
//     ::quint_oracle::__record_outcome(&__quint_ret, __quint_guard);
//     __quint_ret
// }
// ```
//
// The closure (or async block) turns `?` and early `return`s into a value
// the wrapper can inspect; a panic bypasses it and reports through the
// guard's `Drop`. The wrapper never branches on this crate's own (host!)
// configuration: `__register_for`/`__record_outcome` are inert stubs in the
// customer's build when the `enabled` feature is off.
#[proc_macro_attribute]
pub fn test(attr: TokenStream, item: TokenStream) -> TokenStream {
    if !attr.is_empty() {
        return compile_err("#[quint_oracle::test] takes no arguments");
    }
    let mut toks: Vec<TokenTree> = item.into_iter().collect();

    // The body is the trailing brace group; attribute arguments live inside
    // `#[…]` bracket groups, so this flat scan never confuses their contents
    // for the body or the signature.
    let body = match toks.pop() {
        Some(TokenTree::Group(group)) if group.delimiter() == Delimiter::Brace => group,
        _ => return compile_err("#[quint_oracle::test] expects a function with a body"),
    };
    let Some(fn_pos) = toks.iter().position(|tok| is_ident(tok, "fn")) else {
        return compile_err("#[quint_oracle::test] expects a function");
    };
    let name = match toks.get(fn_pos + 1) {
        Some(TokenTree::Ident(ident)) => ident.to_string(),
        _ => return compile_err("#[quint_oracle::test] expects a named function"),
    };
    let is_async = toks[..fn_pos].iter().any(|tok| is_ident(tok, "async"));
    let has_test_attr =
        attr_paths(&toks[..fn_pos]).any(|path| path.split("::").last() == Some("test"));

    let mut out = TokenStream::new();
    // An async fn needs its runtime attribute (#[tokio::test], placed below
    // this one) to be a test; a bare #[test] on it would not compile.
    if !has_test_attr && !is_async {
        out.extend(tokens("#[test]"));
    }
    out.extend(toks);
    out.extend([TokenTree::Group(wrapped_body(&name, is_async, body))]);
    out
}

fn wrapped_body(name: &str, is_async: bool, body: Group) -> Group {
    let mut wrap = if is_async {
        tokens("async move")
    } else {
        tokens("move ||")
    };
    wrap.extend([TokenTree::Group(body)]);
    let wrapped = TokenTree::Group(Group::new(Delimiter::Parenthesis, wrap));

    let mut inner = tokens(&format!(
        "let __quint_guard = \
             ::quint_oracle::__register_for(::core::module_path!(), \"{name}\");
         let __quint_ret ="
    ));
    inner.extend([wrapped]);
    inner.extend(tokens(if is_async { ".await;" } else { "();" }));
    inner.extend(tokens(
        "::quint_oracle::__record_outcome(&__quint_ret, __quint_guard);
         __quint_ret",
    ));
    Group::new(Delimiter::Brace, inner)
}

/// The leading path of each `#[…]` attribute among `toks` — `"test"`,
/// `"tokio::test"`, `"::core::prelude::v1::test"`.
fn attr_paths(toks: &[TokenTree]) -> impl Iterator<Item = String> + '_ {
    toks.windows(2).filter_map(|pair| match pair {
        [TokenTree::Punct(punct), TokenTree::Group(group)]
            if punct.as_char() == '#' && group.delimiter() == Delimiter::Bracket =>
        {
            Some(leading_path(group.stream()))
        }
        _ => None,
    })
}

fn leading_path(stream: TokenStream) -> String {
    let mut path = String::new();
    for tok in stream {
        match tok {
            TokenTree::Ident(ident) => path.push_str(&ident.to_string()),
            TokenTree::Punct(punct) if punct.as_char() == ':' => path.push(':'),
            _ => break,
        }
    }
    path
}

fn is_ident(tok: &TokenTree, name: &str) -> bool {
    matches!(tok, TokenTree::Ident(ident) if ident.to_string() == name)
}

fn tokens(src: &str) -> TokenStream {
    TokenStream::from_str(src).expect("fixed token snippets always lex")
}

fn compile_err(message: &str) -> TokenStream {
    tokens(&format!("::core::compile_error!({message:?});"))
}
