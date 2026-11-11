use super::*;

fn texts(source: &str) -> Vec<String> {
    render(source)
        .variants
        .into_iter()
        .map(|v| v.text.unwrap_or_else(|e| format!("error: {e}")))
        .collect()
}

#[test]
fn each_branch_of_a_chain_renders_once() {
    let source = "{% if a %}1{% elif b %}2{% else %}3{% endif %}|{% if c -%} x {%- endif %}";
    let rendering = render(source);
    assert_eq!(rendering.branches.chains, [3, 2]);
    assert!(!rendering.sampled);
    assert_eq!(texts(source), ["1|x", "2|x", "3|x", "1|", "2|", "3|"]);
    assert_eq!(rendering.variants[4].label, "branches 1.1");
}

#[test]
fn loops_run_zero_one_and_three_times() {
    let source = "{% for x in ['a', 'b'] %}{{ x }},{% else %}none{% endfor %}";
    assert_eq!(texts(source), ["none", "a,", "a,b,a,"]);
    // A loop over a name the template does not define runs over stubs.
    assert_eq!(
        texts("{% for e in this.events %}{{ e }} {% endfor %}"),
        [
            "",
            "this_events0 ",
            "this_events0 this_events1 this_events2 "
        ]
    );
    // A loop with a filter runs as written.
    let filtered = render("{% for x in [1, 2, 3] if x > 1 %}{{ x }}{% endfor %}");
    assert_eq!(filtered.branches.loops, 0);
    assert_eq!(filtered.variants.len(), 1);
    assert_eq!(filtered.variants[0].text.as_deref(), Ok("23"));
}

#[test]
fn a_condition_on_the_loop_is_decided_in_each_pass() {
    let source =
        "{% for c in cols %}{{ c }}{% if not loop.last %},{% elif x %};{% endif %}{% endfor %}";
    let rendering = render(source);
    assert_eq!(rendering.branches.chains, Vec::<usize>::new());
    assert_eq!(texts(source), ["", "cols0;", "cols0,cols1,cols2;"]);
}

#[test]
fn many_branches_are_sampled() {
    let source = "{% if a %}a{% endif %}".repeat(6);
    let rendering = render(&source);
    assert_eq!(rendering.combinations, 64);
    assert!(rendering.sampled);
    assert_eq!(rendering.variants.len(), MAX_COMBINATIONS);
    assert_eq!(rendering.variants[0].text.as_deref(), Ok("aaaaaa"));
    assert_eq!(rendering.variants[1].text.as_deref(), Ok(""));
}

#[test]
fn chum_templates_render_with_stub_values_and_filters() {
    let source = "{% if env.tier == \"prod\" %}\nDROP TABLE {{ env.db | sql_ident }}.t;\n{% endif %}\n\
                  SELECT {{ this.cutoff | sql_string }}, {{ this.at | sql_datetime }};\n";
    assert_eq!(
        texts(source),
        [
            "\nDROP TABLE `env_db`.t;\n\nSELECT 'this_cutoff', toDateTime('2026-11-10 08:00:00', 'UTC');\n",
            "\nSELECT 'this_cutoff', toDateTime('2026-11-10 08:00:00', 'UTC');\n",
        ]
    );
}

#[test]
fn dbt_models_render_to_sql() {
    let source = "{{ config(materialized='incremental') }}\n\
                  select * from {{ ref('orders') }} join {{ source('raw', 'events') }} using (id)\n\
                  {% if is_incremental() %}where ts > (select max(ts) from {{ this }}){% endif %}\n\
                  {{ dbt_utils.generate_surrogate_key(['a']) }}";
    assert_eq!(
        texts(source)[0],
        "\nselect * from orders join raw.events using (id)\n\
         where ts > (select max(ts) from this)\ndbt_utils_generate_surrogate_key"
    );
}

#[test]
fn macros_and_set_run_as_written() {
    let source = "{% macro m(x) -%} f({{ x }}) {%- endmacro %}{% set y = 2 %}{{ m(y) }}";
    assert_eq!(texts(source), ["f(2)"]);
}

#[test]
fn a_template_that_does_not_compile_has_one_failing_variant() {
    let rendering = render("{% if x %}");
    assert_eq!(rendering.variants.len(), 1);
    assert!(rendering.variants[0].text.is_err());
}

#[test]
fn the_rendered_sql_is_compared_token_by_token() {
    assert_eq!(compare_sql("select a  from t", "SELECT a\nFROM t"), Ok(()));
    // An identifier changes case, a keyword is printed in a case that is
    // not its canonical one, and tokens join.
    assert!(compare_sql("select A", "select a").is_err());
    assert!(compare_sql("SELECT 1", "Select 1").is_err());
    assert!(compare_sql("select - -1", "select --1").is_err());
    // The literal left of `::` is read as written.
    assert!(compare_sql("select [1, 2]::String", "select [1,2]::String").is_err());
    assert_eq!(
        compare_sql("select [1, 2]::String", "select\n    [1, 2]::String"),
        Ok(())
    );
    // Not the arguments of a call, nor an array of expressions.
    assert_eq!(
        compare_sql("select f(1,  2)::String", "select f(1, 2)::String"),
        Ok(())
    );
    assert_eq!(
        compare_sql("select [a,  2]::String", "select [a, 2]::String"),
        Ok(())
    );
    // A line comment's trailing whitespace is not its text.
    assert_eq!(compare_sql("select 1 -- c  \n", "select 1 -- c\n"), Ok(()));
}
