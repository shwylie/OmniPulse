// A setting's note can describe both states of its switch (data-on, data-off): it shows the one the
// switch is in
function stateNotes() {
  document.querySelectorAll(".note[data-on]").forEach((n) => {
    const row = n.closest(".row");
    const box = row && row.querySelector('input[type="checkbox"]');
    if (box) n.textContent = box.checked ? n.dataset.on : n.dataset.off;
  });
}
document.addEventListener("change", (e) => { if (e.target && e.target.type === "checkbox") stateNotes(); });
