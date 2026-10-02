# Woodshop implementation

Follow spec.md. Rust/Axum/SQLx/Askama/HTMX only. No live AI calls in tests.
Maximum three concurrent agents; no nested delegation. Work only in assigned files.
Do not read or print secrets. Parameterize SQL, escape HTML, fail closed on authorization.
Root owns integration, configuration, acceptance, and PLAN.md.
Agents commit only assigned files if git writes are permitted; otherwise report paths.

