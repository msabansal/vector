use std::collections::BTreeMap;

use compiler::{state, Resolved};
use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use indoc::indoc;
use vector_common::TimeZone;
use vrl::{Runtime, Value};

struct Source {
    name: &'static str,
    code: &'static str,
}

use vrl_stdlib::{
    uuid_v4, vrl_fn_downcase as downcase, vrl_fn_string as string, vrl_fn_upcase as upcase,
};

#[inline(never)]
#[no_mangle]
pub extern "C" fn vrl_fn_uuid_v4(resolved: &mut Resolved) {
    println!("yo? uuid?");
}

extern "C" {
    fn vrl_fn_upcase(value: *mut Resolved, resolved: *mut Resolved);
}

static SOURCES: [Source; 10] = [
    Source {
        name: "8",
        code: indoc! {r#"
            123
        "#},
    },
    Source {
        name: "7",
        code: indoc! {r#"
            uuid_v4()
        "#},
    },
    Source {
        name: "6",
        code: indoc! {r#"
            .hostname = "vector"

            if .status == "warning" {
                .thing = upcase(.hostname)
            } else if .status == "notice" {
                .thung = downcase(.hostname)
            } else {
                .nong = upcase(.hostname)
            }
        "#},
    },
    Source {
        name: "5",
        code: indoc! {r#"
            .foo == "hi"
        "#},
    },
    Source {
        name: "4",
        code: indoc! {r#"
            derp = "hi!"
        "#},
    },
    Source {
        name: "3",
        code: indoc! {r#"
            .derp = "hi!"
        "#},
    },
    Source {
        name: "2",
        code: indoc! {r#"
            .derp
        "#},
    },
    Source {
        name: "1",
        code: indoc! {r#"
            .
        "#},
    },
    Source {
        name: "parse_json",
        code: indoc! {r#"
            x = parse_json!(s'{"noog": "nork"}')
            x.noog
        "#},
    },
    Source {
        name: "simple",
        code: indoc! {r#"
            .hostname = "vector"

            if .status == "warning" {
                .thing = upcase(.hostname)
            } else if .status == "notice" {
                .thung = downcase(.hostname)
            } else {
                .nong = upcase(.hostname)
            }

            .matches = { "name": .message, "num": "2" }
            .origin, .err = .hostname + "/" + .matches.name + "/" + .matches.num
        "#},
    },
];

#[inline(never)]
#[no_mangle]
pub extern "C" fn derp() {
    println!("derp'n");
}

fn benchmark_kind_display(c: &mut Criterion) {
    derp();
    downcase(&mut Ok(Value::Null), &mut Ok(Value::Null));
    string(&mut Ok(Value::Null), &mut Ok(Value::Null));
    unsafe { vrl_fn_uuid_v4(&mut Ok(Value::Null)) };
    unsafe { vrl_fn_upcase(&mut Ok(Value::Null), &mut Ok(Value::Null)) };
    upcase(&mut Ok(Value::Null), &mut Ok(Value::Null));

    /*
    {
        use inkwell::context::Context;
        use inkwell::targets::{InitializationConfig, Target};
        use inkwell::OptimizationLevel;
        Target::initialize_native(&InitializationConfig::default()).unwrap();
        let context = Context::create();
        let module = context.create_module("test");
        let builder = context.create_builder();

        // Set up the function signature
        let double = context.f64_type();
        let sig = double.fn_type(&[], false);

        // Add the function to our module
        let f = module.add_function("test_fn", sig, None);
        let b = context.append_basic_block(f, "entry");
        builder.position_at_end(b);

        let function_name = "derp".to_owned();
        let function_type = context.void_type().fn_type(&[], false);
        let fn_impl = module.add_function(&function_name, function_type, None);
        builder.build_call(fn_impl, &[], &function_name);

        {
            let function_name = "vrl_fn_uuid_v4".to_owned();
            let function_type = context.void_type().fn_type(&[], false);
            let fn_impl = module.add_function(&function_name, function_type, None);
            builder.build_call(fn_impl, &[], &function_name);
        }

        // Insert a return statement
        let ret = double.const_float(64.0);
        builder.build_return(Some(&ret));

        println!("{}", module.print_to_string().to_string());

        // create the JIT engine
        let mut ee = module
            .create_jit_execution_engine(OptimizationLevel::None)
            .unwrap();

        // fetch our JIT'd function and execute it
        unsafe {
            let test_fn = ee
                .get_function::<unsafe extern "C" fn() -> f64>("test_fn")
                .unwrap();
            let return_value = test_fn.call();
            assert_eq!(return_value, 64.0);
        }
    }
    */

    let mut group = c.benchmark_group("vrl/runtime");
    for source in &SOURCES {
        let state = state::Runtime::default();
        let runtime = Runtime::new(state);
        let tz = TimeZone::default();
        let functions = vrl_stdlib::all();
        let mut state = vrl::state::Compiler::new();
        let program = vrl::compile_with_state(source.code, &functions, &mut state).unwrap();
        let vm = runtime
            .compile(functions, &program, Default::default())
            .unwrap();
        let builder = vrl::llvm::Builder::new().unwrap();
        let context = builder.compile(&state, &program).unwrap();
        context.optimize();
        let execute = context.get_jit_function().unwrap();

        {
            println!("yo");
            let mut obj = Value::Object(BTreeMap::default());
            let mut context = core::Context {
                target: &mut obj,
                timezone: &tz,
            };
            let mut result = Ok(Value::Null);
            println!("bla");
            unsafe { execute.call(&mut context, &mut result) };
            println!("derp");
        }

        group.bench_with_input(
            BenchmarkId::new("LLVM", source.name),
            &execute,
            |b, execute| {
                b.iter_with_setup(
                    || Value::Object(BTreeMap::default()),
                    |mut obj| {
                        {
                            let mut context = core::Context {
                                target: &mut obj,
                                timezone: &tz,
                            };
                            let mut result = Ok(Value::Null);
                            unsafe { execute.call(&mut context, &mut result) };
                        }
                        obj // Return the obj so it doesn't get dropped.
                    },
                )
            },
        );

        group.bench_with_input(BenchmarkId::new("VM", source.name), &vm, |b, vm| {
            let state = state::Runtime::default();
            let mut runtime = Runtime::new(state);
            b.iter_with_setup(
                || Value::Object(BTreeMap::default()),
                |mut obj| {
                    let _ = black_box(runtime.run_vm(vm, &mut obj, &tz));
                    runtime.clear();
                    obj // Return the obj so it doesn't get dropped.
                },
            )
        });

        group.bench_with_input(BenchmarkId::new("Ast", source.name), &(), |b, _| {
            let state = state::Runtime::default();
            let mut runtime = Runtime::new(state);
            b.iter_with_setup(
                || Value::Object(BTreeMap::default()),
                |mut obj| {
                    let _ = black_box(runtime.resolve(&mut obj, &program, &tz));
                    runtime.clear();
                    obj
                },
            )
        });
    }
}

criterion_group!(name = vrl_compiler_kind;
                 config = Criterion::default();
                 targets = benchmark_kind_display);
criterion_main!(vrl_compiler_kind);
