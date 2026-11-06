//! Deterministic sample content for the prototypes.

/// A plausible Rust source file of about `lines` lines.
pub fn rust_source(lines: usize, variant: usize) -> String {
    let mut out = String::from(
        "//! Orders: commands, events and the aggregate that enforces them.\n\nuse std::collections::HashMap;\n\n",
    );
    let mut n = 0;
    while out.lines().count() < lines {
        let tweak = if variant > 0 && n % 7 == 3 {
            " // reviewed"
        } else {
            ""
        };
        let limit = if variant > 0 && n % 11 == 5 { 250 } else { 100 };
        out.push_str(&format!(
            "/// Handles step {n} of the order workflow.\n\
             pub fn handle_step_{n}(orders: &mut HashMap<u64, Order>, id: u64) -> Result<(), OrderError> {{\n\
             \x20   let order = orders.get_mut(&id).ok_or(OrderError::NotFound(id))?;{tweak}\n\
             \x20   if order.lines.len() > {limit} {{\n\
             \x20       return Err(OrderError::TooLarge {{ id, lines: order.lines.len() }});\n\
             \x20   }}\n\
             \x20   order.status = Status::Step{n};\n\
             \x20   let note = format!(\"moved {{}} to step {n} 🚚\", order.reference);\n\
             \x20   order.history.push(note);\n\
             \x20   Ok(())\n\
             }}\n\n"
        ));
        if variant > 0 && n % 13 == 8 {
            out.push_str(&format!("const STEP_{n}_RETRIES: u32 = 3;\n\n"));
        }
        n += 1;
    }
    out
}

/// A long assistant reply (~50 KB) in the format the model returns: prose, lists, tables,
/// code fences and `### FILE:` blocks.
pub fn long_reply() -> String {
    let mut out = String::new();
    let mut i = 0;
    while out.len() < 50_000 {
        out.push_str(&format!(
            "## Step {i}: cancel shipped orders\n\n\
             `Order.Cancel()` now **refuses** shipped orders and records *why* in the history. \
             This keeps the aggregate consistent with [the spec](https://example.com/spec).\n\n\
             - Validates the order exists\n- Rejects `Status::Shipped`\n  - nested detail with `inline code`\n- Emits `OrderCancelled`\n\n\
             | File | Change | Tokens |\n| --- | --- | ---: |\n| `src/order.rs` | modified | {} |\n| `src/events.rs` | added | {} |\n\n\
             ```rust\nimpl Order {{\n    pub fn cancel(&mut self, reason: &str) -> Result<(), OrderError> {{\n        if self.status == Status::Shipped {{\n            return Err(OrderError::AlreadyShipped(self.id));\n        }}\n        self.status = Status::Cancelled;\n        self.history.push(format!(\"cancelled: {{reason}}\"));\n        Ok(())\n    }}\n}}\n```\n\n\
             > Note: run the tests after saving; nothing is written until you press **Save**.\n\n",
            1200 + i * 17,
            300 + i * 5
        ));
        i += 1;
    }
    out
}
