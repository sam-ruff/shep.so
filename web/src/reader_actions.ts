/** Retain same-message reader actions and same-scope row controls, including
 * their structural ancestors. Even momentary detachment cancels a native press.
 * Stable preference controls keep native focus and held presses as well. */
export function renderReaderTree(root: HTMLElement, next: HTMLElement) {
  const pairs = new Map<Element, Element>(),
    reverse = new Map<Element, Element>();
  const leaves = new Set<Element>();
  const path = (leaf: Element, ancestor: Element) => {
    const result: Element[] = [];
    for (let node: Element | null = leaf; node; node = node.parentElement) {
      result.unshift(node);
      if (node === ancestor) return result;
    }
    return [];
  };
  const retain = (old: Element, fresh: Element) => {
    const before = path(old, root),
      after = path(fresh, next);
    if (
      before.length !== after.length ||
      before.some(
        (node, i) =>
          node.tagName !== after[i].tagName ||
          (pairs.has(node) && pairs.get(node) !== after[i]) ||
          (reverse.has(after[i]) && reverse.get(after[i]) !== node),
      )
    )
      return;
    before.forEach((node, i) => {
      pairs.set(node, after[i]);
      reverse.set(after[i], node);
    });
    leaves.add(old);
  };
  const stable = new Map(
    [...next.querySelectorAll<HTMLElement>("[data-stable]")].map((n) => [
      n.dataset.stable,
      n,
    ]),
  );
  for (const old of root.querySelectorAll<HTMLElement>("[data-stable]")) {
    const fresh = stable.get(old.dataset.stable);
    if (fresh) retain(old, fresh);
  }
  const oldActions = root.querySelector<HTMLElement>(".reader-actions"),
    newActions = next.querySelector<HTMLElement>(".reader-actions");
  if (
    oldActions &&
    newActions &&
    oldActions.dataset.message === newActions.dataset.message
  )
    retain(oldActions, newActions);
  const oldRows = root.querySelector<HTMLElement>(".rows"),
    newRows = next.querySelector<HTMLElement>(".rows");
  if (oldRows && newRows && oldRows.dataset.scope === newRows.dataset.scope) {
    const controls = new Map(
      [...newRows.querySelectorAll<HTMLElement>("[data-focus]")].map((n) => [
        n.dataset.focus,
        n,
      ]),
    );
    for (const old of oldRows.querySelectorAll<HTMLElement>("[data-focus]")) {
      const fresh = controls.get(old.dataset.focus);
      if (fresh) retain(old, fresh);
    }
  }
  const attributes = (old: Element, fresh: Element) => {
    for (const a of [...old.attributes])
      if (!fresh.hasAttribute(a.name)) old.removeAttribute(a.name);
    for (const a of fresh.attributes)
      if (old.getAttribute(a.name) !== a.value)
        old.setAttribute(a.name, a.value);
  };
  // These retained control subtrees use property listeners only. Rebind them to
  // current metadata while retaining their SVG/text hit targets where possible.
  const control = (old: Element, fresh: Element) => {
    attributes(old, fresh);
    if (old instanceof HTMLElement && fresh instanceof HTMLElement) {
      old.onclick = fresh.onclick;
      old.ondblclick = fresh.ondblclick;
      old.onkeydown = fresh.onkeydown;
      old.onblur = fresh.onblur;
    }
    if (old instanceof HTMLButtonElement && fresh instanceof HTMLButtonElement)
      old.disabled = fresh.disabled;
    if (old instanceof HTMLInputElement && fresh instanceof HTMLInputElement) {
      old.checked = fresh.checked;
      old.disabled = fresh.disabled;
    }
    const children = [...old.childNodes],
      replacements = [...fresh.childNodes];
    if (
      children.length !== replacements.length ||
      children.some(
        (node, i) =>
          node.nodeType !== replacements[i].nodeType ||
          (node instanceof Element &&
            node.tagName !== (replacements[i] as Element).tagName),
      )
    ) {
      old.replaceChildren(...replacements);
      return;
    }
    children.forEach((node, i) => {
      if (node instanceof Element) control(node, replacements[i] as Element);
      else if (node.textContent !== replacements[i].textContent)
        node.textContent = replacements[i].textContent;
    });
  };
  const update = (old: Element, fresh: Element) => {
    if (leaves.has(old)) {
      control(old, fresh);
      return;
    }
    if (old !== root) attributes(old, fresh);
    if (old instanceof HTMLElement && fresh instanceof HTMLElement)
      old.onkeydown = fresh.onkeydown;
    for (const child of [...old.childNodes])
      if (
        !(child instanceof Element) ||
        !pairs.has(child) ||
        pairs.get(child)!.parentElement !== fresh
      )
        child.remove();
    let cursor = old.firstChild;
    for (const node of [...fresh.childNodes]) {
      const retained = node instanceof Element ? reverse.get(node) : undefined;
      if (retained?.parentElement === old) {
        // Unchanged row order never removes or moves any retained branch.
        // An actual sort/membership reordering may move a surviving row.
        if (retained !== cursor) old.insertBefore(retained, cursor);
        update(retained, node as Element);
        cursor = retained.nextSibling;
      } else old.insertBefore(node, cursor);
    }
  };
  if (pairs.has(root)) update(root, next);
  else root.replaceChildren(...next.childNodes);
}
