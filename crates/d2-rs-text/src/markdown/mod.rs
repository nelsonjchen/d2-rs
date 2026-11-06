// SPDX-License-Identifier: MPL-2.0
//! Pinned Goldmark translation used by D2's Markdown processing.
pub mod arena;
pub mod block_reader;
pub mod bytes;
pub mod closure;
pub mod context;
pub mod delimiter;
pub mod inline;
pub mod link;
pub mod link_safety;
pub mod reader;
pub mod reader_api;
pub mod text;

mod case_folding;

pub mod reference;

pub mod link_state;

pub mod link_parser;

pub mod autolink;

pub mod raw_html;

pub mod tag_matcher;
mod tag_program;

pub mod dispatcher;

pub mod paragraph;

pub mod table;

pub mod block;

pub mod code_block;

pub mod fenced_code;

pub mod heading;

pub mod list;

pub mod document;

mod entities;
pub mod html;
pub mod html_block;
pub mod plain_text;
pub mod substitute;

pub mod html_names;

#[doc(hidden)]
pub mod html_tree;
