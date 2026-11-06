use super::*;
#[test]
fn conditional_descriptions_render_per_harness_and_preserve_unrelated_source() -> Result<()> {
    let original = "---\r\nname: sample\r\ndescription: 'Review 🐿' # keep\r\nlicense: MIT\r\n---\r\nBody unchanged.\r\n";
    let metadata = "# untouched\nenabled = true\n";
    let draft = edit_draft(
        original,
        metadata,
        DraftEdit::DescriptionSection {
            edit: DescriptionEdit::Append {
                value: "Luna, you must ALWAYS load this, no exceptions".into(),
                selectors: BTreeMap::from([("harness".into(), vec!["codex".into()])]),
            },
        },
    )?;
    assert_eq!(draft.metadata, metadata);
    assert!(
        draft
            .markdown
            .ends_with(" # keep\r\nlicense: MIT\r\n---\r\nBody unchanged.\r\n")
    );
    let view = editor_view(&draft.markdown, metadata)?;
    assert_eq!(view.description_sections.len(), 3);
    assert_eq!(
        view.description_sections[1].text,
        "Luna, you must ALWAYS load this, no exceptions"
    );
    for harness in ["codex", "claude"] {
        let context: toml::Table = [("harness".into(), harness.into())].into_iter().collect();
        let rendered =
            crate::template::render(&draft.markdown, &context, true)?.replace("\r\n", "\n");
        let (yaml, _) = crate::template::skill_frontmatter(&rendered, "sample")?;
        assert_eq!(
            yaml["description"].as_str().unwrap().contains("Luna"),
            harness == "codex"
        );
        if harness == "claude" {
            assert_eq!(yaml["description"].as_str(), Some("Review 🐿"));
        }
        assert!(!rendered.contains("<!--"));
    }
    // Typing repeatedly into a section addresses the current section, not stale offsets.
    let changed = edit_draft(
        &draft.markdown,
        metadata,
        DraftEdit::DescriptionSection {
            edit: DescriptionEdit::Text {
                index: 0,
                value: "Longer shared text: \"quoted\" 🐿".into(),
            },
        },
    )?;
    let changed = edit_draft(
        &changed.markdown,
        metadata,
        DraftEdit::DescriptionSection {
            edit: DescriptionEdit::Text {
                index: 1,
                value: "Updated: \"Luna\"".into(),
            },
        },
    )?;
    let changed = edit_draft(
        &changed.markdown,
        metadata,
        DraftEdit::DescriptionSection {
            edit: DescriptionEdit::Only {
                index: 1,
                selectors: BTreeMap::from([("harness".into(), vec!["claude".into()])]),
            },
        },
    )?;
    assert_eq!(
        editor_view(&changed.markdown, metadata)?.description_sections[1]
            .selectors
            .as_ref()
            .unwrap()["harness"],
        vec!["claude"]
    );
    let unwrapped = edit_draft(
        &changed.markdown,
        metadata,
        DraftEdit::DescriptionSection {
            edit: DescriptionEdit::RemoveOnly { index: 1 },
        },
    )?;
    assert_eq!(
        editor_view(&unwrapped.markdown, metadata)?.description,
        "Longer shared text: \"quoted\" 🐿 Updated: \"Luna\""
    );
    Ok(())
}

#[test]
fn broken_description_conditions_remain_openable_for_source_repair() -> Result<()> {
    let source =
        "---\nname: sample\ndescription: 'Shared <!-- only harness=codex -->broken'\n---\nBody\n";
    let view = editor_view(source, "")?;
    assert!(view.description_error.is_some());
    assert!(view.description_sections.is_empty());
    assert!(
        edit_draft(
            source,
            "",
            DraftEdit::DescriptionSection {
                edit: DescriptionEdit::Text {
                    index: 0,
                    value: "oops".into()
                }
            }
        )
        .is_err()
    );
    Ok(())
}
#[test]
fn structured_edits_preserve_unrelated_bytes_and_noops() {
    let md = "---\r\nname: sample\r\ndescription: 'old'\r\n# retained\r\nlicense: MIT\r\n---\r\n🐿 Body\r\n";
    let meta = "# comment\r\nenabled = true # keep\r\n[target]\r\nharnesses = ['codex']\r\n";
    assert_eq!(
        edit_draft(
            md,
            meta,
            DraftEdit::Description {
                value: "old".into()
            }
        )
        .unwrap()
        .markdown,
        md
    );
    let result = edit_draft(
        md,
        meta,
        DraftEdit::Description {
            value: "new".into(),
        },
    )
    .unwrap();
    assert_eq!(
        result.markdown,
        md.replace("description: 'old'", "description: \"new\"")
    );
    assert_eq!(result.metadata, meta);
    assert_eq!(
        patch_toml(meta, &["enabled".into()], Some(&true.into())).unwrap(),
        meta
    );
    let edited = patch_toml(meta, &["enabled".into()], Some(&false.into())).unwrap();
    assert!(
        edited.contains("# comment") && edited.contains("# keep") && edited.contains("['codex']")
    );
}
#[test]
fn conditions_and_values_share_parser_and_utf16_boundaries() {
    let md = "🐿 hello\r\n";
    let result = edit_draft(
        md,
        "",
        DraftEdit::Only {
            from: 3,
            to: 8,
            selectors: BTreeMap::from([("harness".into(), vec!["codex".into()])]),
            existing: false,
        },
    )
    .unwrap();
    assert_eq!(
        result.markdown,
        "🐿 <!-- only harness=codex -->\r\nhello\r\n<!-- /only -->\r\n"
    );
    let removed = edit_draft(&result.markdown, "", DraftEdit::RemoveOnly { from: 3 }).unwrap();
    assert_eq!(removed.markdown, "🐿 \r\nhello\r\n\r\n");
    for (source, expected) in [
        (
            "first\nsecond\n",
            "<!-- only harness=codex -->\nfirst\n<!-- /only -->\nsecond\n",
        ),
        (
            "first\r\nsecond\r\n",
            "<!-- only harness=codex -->\r\nfirst\r\n<!-- /only -->\r\nsecond\r\n",
        ),
        (
            "first\nsecond\r\n",
            "<!-- only harness=codex -->\r\nfirst\r\n<!-- /only -->\nsecond\r\n",
        ),
    ] {
        let line = source.find("second").unwrap();
        let wrapped = edit_draft(
            source,
            "",
            DraftEdit::Only {
                from: 0,
                to: line,
                selectors: BTreeMap::from([("harness".into(), vec!["codex".into()])]),
                existing: false,
            },
        )
        .unwrap();
        assert_eq!(wrapped.markdown, expected);
    }
    assert!(
        edit_draft(
            md,
            "",
            DraftEdit::Variable {
                from: 1,
                to: 1,
                name: "paths.archive".into()
            }
        )
        .is_err()
    );
}
