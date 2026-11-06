use super::*;
#[test]
fn yaml_scalar_ranges_keep_inline_comments_flow_mappings_and_multiline_fields() -> Result<()> {
    for (header, expected) in [
        (
            "name: sample\ndescription: 'old' # keep\nlicense: MIT\n",
            "name: sample\ndescription: \"new\" # keep\nlicense: MIT\n",
        ),
        (
            "{name: sample, description: old, license: MIT}\n",
            "{name: sample, description: \"new\", license: MIT}\n",
        ),
        (
            "name: sample\ndescription: >-\n  old words\n# keep\nlicense: MIT\n",
            "name: sample\ndescription: \"new\"\n# keep\nlicense: MIT\n",
        ),
    ] {
        let md = format!("---\n{header}---\n🐿 body\n");
        assert_eq!(
            yaml_field(&md, "description", "new")?,
            format!("---\n{expected}---\n🐿 body\n")
        );
    }
    Ok(())
}
#[test]
fn toml_single_field_keeps_every_other_byte() -> Result<()> {
    let source = "# header\r\nenabled = true # retained\r\n[target]\r\nharnesses = ['codex']\r\n";
    assert_eq!(
        patch_toml(source, &["enabled".into()], Some(&false.into()))?,
        source.replace("true", "false")
    );
    Ok(())
}
