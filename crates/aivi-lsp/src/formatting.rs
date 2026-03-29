use tower_lsp::lsp_types::TextEdit;

/// Format a document and return LSP text edits.
pub fn format_document(
    db: &aivi_query::RootDatabase,
    file: aivi_query::SourceFile,
) -> Option<Vec<TextEdit>> {
    let parsed = aivi_query::parsed_file(db, file);
    let source = parsed.source_arc();
    let formatted = aivi_query::format_file(db, file)?;

    if formatted == source.text() {
        return Some(Vec::new());
    }

    Some(vec![TextEdit {
        range: crate::diagnostics::lsp_range(source.span_to_lsp_range(source.full_span().span())),
        new_text: formatted,
    }])
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use aivi_query::{RootDatabase, SourceFile};

    use super::format_document;

    #[test]
    fn format_document_formats_record_row_transform_types() {
        let db = RootDatabase::new();
        let file = SourceFile::new(
            &db,
            PathBuf::from("record-row-types.aivi"),
            concat!(
                "type User={id:Int,name:Text,createdAt:Text}\n",
                "type Public=User |> Pick (id,createdAt) |> Rename {createdAt:created_at}\n",
            )
            .to_owned(),
        );

        let edits = format_document(&db, file).expect("formatting should succeed");
        assert_eq!(edits.len(), 1);
        assert_eq!(
            edits[0].new_text,
            concat!(
                "type User = {\n",
                "    id: Int,\n",
                "    name: Text,\n",
                "    createdAt: Text\n",
                "}\n",
                "\n",
                "type Public = User |> Pick (id, createdAt) |> Rename { createdAt: created_at }\n",
            )
        );
    }
}
