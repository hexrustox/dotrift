use crate::{ast::Node, error::ParseError, scanner::Token};

/// Assembles tokens into AST nodes. This slice passes text tokens through
/// unchanged; tags are not recognized yet.
pub(crate) fn parse(tokens: Vec<Token>) -> std::result::Result<Vec<Node>, ParseError> {
    Ok(tokens
        .into_iter()
        .map(|token| match token {
            Token::Text(range) => Node::Text(range),
        })
        .collect())
}
