use std::fmt;

use crate::{
    expression::{Expr, Resolved},
    vm::OpCode,
    Context, Expression, State, TypeDef, Value,
};

#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    inner: Vec<Expr>,
}

impl Block {
    pub fn new(inner: Vec<Expr>) -> Self {
        Self { inner }
    }

    pub fn into_inner(self) -> Vec<Expr> {
        self.inner
    }
}

impl Expression for Block {
    fn resolve(&self, ctx: &mut Context) -> Resolved {
        self.inner
            .iter()
            .map(|expr| expr.resolve(ctx))
            .collect::<Result<Vec<_>, _>>()
            .map(|mut v| v.pop().unwrap_or(Value::Null))
    }

    fn type_def(&self, state: &State) -> TypeDef {
        let mut type_defs = self
            .inner
            .iter()
            .map(|expr| expr.type_def(state))
            .collect::<Vec<_>>();

        // If any of the stored expressions is fallible, the entire block is
        // fallible.
        let fallible = type_defs.iter().any(TypeDef::is_fallible);

        // The last expression determines the resulting value of the block.
        let type_def = type_defs.pop().unwrap_or_else(TypeDef::null);

        type_def.with_fallibility(fallible)
    }

    fn compile_to_vm(
        &self,
        vm: &mut crate::vm::Vm,
        state: &mut crate::state::Compiler,
    ) -> Result<(), String> {
        let mut jumps = Vec::new();

        // An empty block should resolve to Null.
        if self.inner.is_empty() {
            let null = vm.add_constant(Value::Null);
            vm.write_opcode(OpCode::Constant);
            vm.write_primitive(null);
        }

        let mut expressions = self.inner.iter().peekable();

        while let Some(expr) = expressions.next() {
            // Write each of the inner expressions
            expr.compile_to_vm(vm, state)?;

            if expressions.peek().is_some() {
                // At the end of each statement (apart from the last one) we need to clean up
                // This involves popping the value remaining on the stack, and jumping to the end
                // of the block if we are in error.
                jumps.push(vm.emit_jump(OpCode::EndStatement));
            }
        }

        // Update all the jumps to jump to the end of the block.
        for jump in jumps {
            vm.patch_jump(jump);
        }

        Ok(())
    }

    #[cfg(feature = "llvm")]
    fn emit_llvm<'ctx>(
        &self,
        state: &crate::state::Compiler,
        ctx: &mut crate::llvm::Context<'ctx>,
    ) -> Result<(), String> {
        let function = ctx.function();
        let block_begin_block = ctx.context().append_basic_block(function, "block_begin");
        ctx.builder().build_unconditional_branch(block_begin_block);
        ctx.builder().position_at_end(block_begin_block);

        let block_end_block = ctx.context().append_basic_block(function, "block_end");
        let block_error_block = ctx.context().append_basic_block(function, "block_error");

        for expr in &self.inner {
            expr.emit_llvm(state, ctx)?;
            let is_err = {
                let fn_ident = "vrl_resolved_is_err";
                let fn_impl = ctx
                    .module()
                    .get_function(fn_ident)
                    .ok_or(format!(r#"failed to get "{}" function"#, fn_ident))?;
                ctx.builder()
                    .build_call(fn_impl, &[ctx.result_ref().into()], fn_ident)
                    .try_as_basic_value()
                    .left()
                    .ok_or(format!(r#"result of "{}" is not a basic value"#, fn_ident))?
                    .try_into()
                    .map_err(|_| format!(r#"result of "{}" is not an int value"#, fn_ident))?
            };

            let block_next_block = ctx.context().append_basic_block(function, "block_next");
            ctx.builder()
                .build_conditional_branch(is_err, block_error_block, block_next_block);
            ctx.builder().position_at_end(block_next_block);
        }

        let block_next_block = ctx.builder().get_insert_block().unwrap();

        ctx.builder().position_at_end(block_error_block);
        ctx.builder().build_unconditional_branch(block_end_block);

        ctx.builder().position_at_end(block_next_block);
        ctx.builder().build_unconditional_branch(block_end_block);

        ctx.builder().position_at_end(block_end_block);

        Ok(())
    }
}

impl fmt::Display for Block {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("{\n")?;

        let mut iter = self.inner.iter().peekable();
        while let Some(expr) = iter.next() {
            f.write_str("\t")?;
            expr.fmt(f)?;
            if iter.peek().is_some() {
                f.write_str("\n")?;
            }
        }

        f.write_str("\n}")
    }
}
