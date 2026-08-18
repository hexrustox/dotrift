use std::collections::{BTreeMap, HashMap};
use std::fmt::Write;

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use templater::Template;
use templater::util::TestRegistry;
use templater::value::{Value, ValueType};

const TOP_LEVEL_VARIABLES: usize = 50;
const TEMPLATE_SECTIONS: usize = 6_000;
const MAX_VALUE_DEPTH: usize = 2;
const SEEDS: [u64; 8] = [
    0x5eed_0001,
    0x5eed_0002,
    0x5eed_0003,
    0x5eed_0004,
    0x5eed_0005,
    0x5eed_0006,
    0x5eed_0007,
    0x5eed_0008,
];

#[derive(Clone)]
struct PathEntry {
    path: String,
    value_type: ValueType,
    top_level: bool,
}

fn random_identifier(rng: &mut StdRng, used: &mut Vec<String>) -> String {
    loop {
        let suffix: String = (0..rng.random_range(4..=10))
            .map(|_| rng.random_range(b'a'..=b'z') as char)
            .collect();
        let identifier = format!("v_{suffix}");
        if !used.iter().any(|name| name == &identifier) {
            used.push(identifier.clone());
            return identifier;
        }
    }
}

fn random_string(rng: &mut StdRng, min: usize, max: usize) -> String {
    (0..rng.random_range(min..=max))
        .map(|_| match rng.random_range(0..12) {
            0 => ' ',
            1 => '\n',
            _ => rng.random_range(b'a'..=b'z') as char,
        })
        .collect()
}

fn random_value(
    rng: &mut StdRng,
    depth: usize,
    path: &str,
    top_level: bool,
    catalog: &mut Vec<PathEntry>,
) -> Value {
    let kind = if depth >= MAX_VALUE_DEPTH {
        rng.random_range(0..3)
    } else {
        rng.random_range(0..5)
    };

    let (value, value_type) = match kind {
        0 => (Value::Str(random_string(rng, 4, 20)), ValueType::Str),
        1 => (
            Value::Int(rng.random_range(-10_000..=10_000)),
            ValueType::Int,
        ),
        2 => (Value::Bool(rng.random_bool(0.5)), ValueType::Bool),
        3 => {
            let length = rng.random_range(1..=4);
            let items = (0..length)
                .map(|index| {
                    let child_path = format!("{path}.{index}");
                    random_value(rng, depth + 1, &child_path, false, catalog)
                })
                .collect();
            (Value::List(items), ValueType::List)
        }
        _ => {
            let length = rng.random_range(1..=4);
            let mut map = BTreeMap::new();
            for index in 0..length {
                let key = format!("field_{index}");
                let child_path = format!("{path}.{key}");
                let value = random_value(rng, depth + 1, &child_path, false, catalog);
                map.insert(key, value);
            }
            (Value::Map(map), ValueType::Map)
        }
    };

    catalog.push(PathEntry {
        path: path.to_owned(),
        value_type,
        top_level,
    });
    value
}

fn build_scope(seed: u64) -> (HashMap<String, Value>, Vec<PathEntry>) {
    let mut rng = StdRng::seed_from_u64(seed);

    loop {
        let mut scope = HashMap::with_capacity(TOP_LEVEL_VARIABLES);
        let mut catalog = Vec::new();
        let mut used = Vec::new();

        for _ in 0..TOP_LEVEL_VARIABLES {
            let name = random_identifier(&mut rng, &mut used);
            let value = random_value(&mut rng, 0, &name, true, &mut catalog);
            scope.insert(name, value);
        }

        let has_bool = catalog
            .iter()
            .any(|entry| entry.top_level && entry.value_type == ValueType::Bool);
        let has_list = catalog
            .iter()
            .any(|entry| entry.top_level && entry.value_type == ValueType::List);
        if has_bool && has_list {
            return (scope, catalog);
        }
    }
}

fn random_path(
    rng: &mut StdRng,
    catalog: &[PathEntry],
    requested_type: Option<ValueType>,
) -> String {
    let candidates: Vec<&PathEntry> = catalog
        .iter()
        .filter(|entry| requested_type.is_none_or(|value_type| entry.value_type == value_type))
        .collect();

    let candidates = if candidates.is_empty() {
        catalog.iter().filter(|entry| entry.top_level).collect()
    } else {
        candidates
    };

    candidates[rng.random_range(0..candidates.len())]
        .path
        .clone()
}

fn random_plain_text(rng: &mut StdRng, output: &mut String) {
    output.push_str(&random_string(rng, 8, 48));
    output.push('\n');
}

fn random_comment(rng: &mut StdRng, output: &mut String) {
    writeln!(output, "{{# {} #}}", random_string(rng, 8, 32)).unwrap();
}

fn random_variable(rng: &mut StdRng, catalog: &[PathEntry], output: &mut String) {
    let requested_type = if rng.random_bool(0.5) {
        None
    } else {
        Some(match rng.random_range(0..5) {
            0 => ValueType::Str,
            1 => ValueType::Int,
            2 => ValueType::Bool,
            3 => ValueType::List,
            _ => ValueType::Map,
        })
    };
    let path = random_path(rng, catalog, requested_type);
    writeln!(output, "{{{{ {path} }}}}").unwrap();
}

fn random_section(rng: &mut StdRng, catalog: &[PathEntry], depth: usize, output: &mut String) {
    let choice_count = if depth >= MAX_VALUE_DEPTH { 3 } else { 5 };
    match rng.random_range(0..choice_count) {
        0 => random_plain_text(rng, output),
        1 => random_comment(rng, output),
        2 => random_variable(rng, catalog, output),
        3 => {
            let condition = random_path(rng, catalog, Some(ValueType::Bool));
            writeln!(output, "{{% if {condition} %}}").unwrap();
            random_section(rng, catalog, depth + 1, output);

            while rng.random_bool(0.5) {
                let condition = random_path(rng, catalog, Some(ValueType::Bool));
                writeln!(output, "{{% elif {condition} %}}").unwrap();
                random_section(rng, catalog, depth + 1, output);
            }

            if rng.random_bool(0.5) {
                output.push_str("{% else %}\n");
                random_section(rng, catalog, depth + 1, output);
            }
            output.push_str("{% end %}\n");
        }
        _ => {
            let iterable = random_path(rng, catalog, Some(ValueType::List));
            writeln!(output, "{{% for _ in {iterable} %}}").unwrap();
            random_section(rng, catalog, depth + 1, output);
            output.push_str("{% end %}\n");
        }
    }
}

fn build_template(seed: u64, catalog: &[PathEntry]) -> String {
    let mut rng = StdRng::seed_from_u64(seed ^ 0xa5a5_a5a5);
    let mut output = String::new();
    for _ in 0..TEMPLATE_SECTIONS {
        random_section(&mut rng, catalog, 0, &mut output);
    }
    output
}

fn build_fixtures() -> Vec<(String, Template, HashMap<String, Value>)> {
    SEEDS
        .iter()
        .map(|seed| {
            let (scope, catalog) = build_scope(*seed);
            let source = build_template(*seed, &catalog);
            let template = Template::from_bytes(source.clone());
            (source, template, scope)
        })
        .collect()
}

fn render_benchmark(c: &mut Criterion) {
    let fixtures = build_fixtures();
    let mut group = c.benchmark_group("render");
    for (fixture_index, (source, template, scope)) in fixtures.iter().enumerate() {
        group.throughput(Throughput::Bytes(source.len() as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(fixture_index),
            &(template, scope),
            |b, (template, scope): &(&Template, &HashMap<String, Value>)| {
                b.iter(|| {
                    template
                        .render(&mut Vec::new(), scope, &TestRegistry)
                        .expect("render failed")
                });
            },
        );
    }
    group.finish();
}

criterion_group!(benches, render_benchmark);
criterion_main!(benches);
