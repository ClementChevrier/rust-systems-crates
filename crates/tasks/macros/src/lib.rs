//! The procedural macro behind [`tasks::task!`](https://docs.rs/tasks). Use it
//! through the `tasks` facade, which also re-exports the enums it expects.

use proc_macro::TokenStream;

mod core;
mod main_macro;

/// Records a task as a markdown file and, when `panic: true`, expands to a
/// `todo!()` pointing at that file. See the `tasks` crate for the syntax.
#[proc_macro]
pub fn task(input: TokenStream) -> TokenStream {
    main_macro::task(input)
}
