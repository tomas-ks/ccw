export function clamp(value, min, max) {
  return Math.min(max, Math.max(min, value));
}

export function pickRegionClientCenter(detail, viewerCanvas) {
  const region = detail?.region;
  if (!region || !viewerCanvas) {
    return null;
  }
  const rect = viewerCanvas.getBoundingClientRect();
  const surfaceWidth = Math.max(1, viewerCanvas.width || rect.width);
  const surfaceHeight = Math.max(1, viewerCanvas.height || rect.height);
  return {
    x: rect.left + ((Number(region.x) + Number(region.width) / 2) / surfaceWidth) * rect.width,
    y: rect.top + ((Number(region.y) + Number(region.height) / 2) / surfaceHeight) * rect.height,
  };
}

export function resolvePropertiesBalloonDom(doc = globalThis.document) {
  return {
    viewport: doc?.querySelector?.(".viewport") || null,
    viewerCanvas: doc?.getElementById?.("viewer-canvas") || null,
    pickAnchorMarker: doc?.getElementById?.("pick-anchor-marker") || null,
    propertiesBalloon: doc?.getElementById?.("properties-balloon") || null,
    propertiesCloseButton: doc?.getElementById?.("properties-close-button") || null,
    propertiesGraphButton: doc?.getElementById?.("properties-graph-button") || null,
    propertiesTitle: doc?.getElementById?.("properties-title") || null,
    propertiesSubtitle: doc?.getElementById?.("properties-subtitle") || null,
    propertiesEmptyState: doc?.getElementById?.("properties-empty-state") || null,
    propertiesCoreSection: doc?.getElementById?.("properties-core-section") || null,
    propertiesCoreGrid: doc?.getElementById?.("properties-core-grid") || null,
    propertiesExtraSection: doc?.getElementById?.("properties-extra-section") || null,
    propertiesExtraGrid: doc?.getElementById?.("properties-extra-grid") || null,
    propertiesRelationsSection: doc?.getElementById?.("properties-relations-section") || null,
    propertiesRelationsList: doc?.getElementById?.("properties-relations-list") || null,
  };
}

export function hidePickAnchorMarker(pickAnchorMarker) {
  if (pickAnchorMarker) {
    pickAnchorMarker.hidden = true;
  }
}

export function applyHiddenPropertiesBalloon({
  propertiesBalloon = null,
  pickAnchorMarker = null,
} = {}) {
  if (propertiesBalloon) {
    propertiesBalloon.hidden = true;
  }
  hidePickAnchorMarker(pickAnchorMarker);
}

export function positionPropertiesBalloonAtClientPoint(
  {
    viewport = null,
    propertiesBalloon = null,
    pickAnchorMarker = null,
  } = {},
  clientX,
  clientY,
  {
    anchored = false,
    marker = false,
  } = {}
) {
  if (!propertiesBalloon || !viewport) {
    return false;
  }

  const viewportRect = viewport.getBoundingClientRect();
  propertiesBalloon.hidden = false;
  const balloonRect = propertiesBalloon.getBoundingClientRect();
  const padding = 12;
  const sideGap = 22;
  const leftBias = 18;
  const localX = clientX - viewportRect.left;
  const localY = clientY - viewportRect.top;

  if (marker && pickAnchorMarker) {
    pickAnchorMarker.hidden = false;
    pickAnchorMarker.style.left = `${Math.round(localX)}px`;
    pickAnchorMarker.style.top = `${Math.round(localY)}px`;
  } else {
    hidePickAnchorMarker(pickAnchorMarker);
  }

  const maxLeft = Math.max(padding, viewportRect.width - balloonRect.width - padding);
  const maxTop = Math.max(padding, viewportRect.height - balloonRect.height - padding);
  let side = "center";
  let left = localX - balloonRect.width / 2 - leftBias;
  if (anchored) {
    const rightLeft = localX + sideGap - leftBias;
    const leftLeft = localX - sideGap - balloonRect.width - leftBias;
    const rightFits = rightLeft + balloonRect.width <= viewportRect.width - padding;
    const leftFits = leftLeft >= padding;
    const rightSpace = viewportRect.width - localX - padding;
    const leftSpace = localX - padding;
    if (rightFits || (!leftFits && rightSpace >= leftSpace)) {
      side = "right";
      left = rightLeft;
    } else {
      side = "left";
      left = leftLeft;
    }
  }

  left = clamp(left, padding, maxLeft);
  propertiesBalloon.style.left = `${Math.round(left)}px`;
  const top = clamp(localY - balloonRect.height / 2, padding, maxTop);
  propertiesBalloon.style.top = `${Math.round(top)}px`;
  if (anchored) {
    const rightEdge = left + balloonRect.width;
    if (localX < left) {
      side = "right";
    } else if (localX > rightEdge) {
      side = "left";
    } else {
      side = localX - left <= rightEdge - localX ? "right" : "left";
    }
  }
  propertiesBalloon.dataset.side = side;
  const arrowY = clamp(localY - top, 18, balloonRect.height - 18);
  propertiesBalloon.style.setProperty("--arrow-y", `${Math.round(arrowY)}px`);
  return true;
}

export function positionPropertiesBalloonFromAnchor(dom = {}, anchor = null) {
  if (
    !anchor ||
    !(
      anchor.kind === "client" ||
      anchor.kind === "viewport-center" ||
      (Number.isFinite(anchor.clientX) && Number.isFinite(anchor.clientY))
    )
  ) {
    applyHiddenPropertiesBalloon(dom);
    return false;
  }
  return positionPropertiesBalloonAtClientPoint(dom, anchor.clientX, anchor.clientY, {
    anchored: Boolean(anchor.anchored),
    marker: Boolean(anchor.marker),
  });
}

export function formatPropertyLabel(label) {
  return String(label || "")
    .replace(/_/g, " ")
    .replace(/([a-z0-9])([A-Z])/g, "$1 $2")
    .replace(/\s+/g, " ")
    .trim();
}

export function propertyValueText(value) {
  if (value === null || value === undefined || value === "") {
    return null;
  }
  return typeof value === "object" ? JSON.stringify(value) : String(value);
}

function ownerDocumentForDom(dom = {}) {
  return (
    dom.propertiesBalloon?.ownerDocument ||
    dom.propertiesCoreGrid?.ownerDocument ||
    globalThis.document
  );
}

function rowLabel(row) {
  return Array.isArray(row) ? row[0] : row?.label;
}

function rowValue(row) {
  return Array.isArray(row) ? row[1] : row?.value;
}

export function createPropertyRow(label, value, doc = globalThis.document) {
  const fragment = doc.createDocumentFragment();
  const dt = doc.createElement("div");
  dt.className = "property-label";
  dt.textContent = formatPropertyLabel(label);
  const dd = doc.createElement("div");
  dd.className = "property-value";
  dd.textContent = value;
  fragment.append(dt, dd);
  return fragment;
}

function createPropertyRows(rows, doc) {
  return (Array.isArray(rows) ? rows : [])
    .map((row) => [rowLabel(row), propertyValueText(rowValue(row))])
    .filter(([, value]) => value !== null)
    .map(([label, value]) => createPropertyRow(label, value, doc));
}

function relationTitle(relation) {
  return (
    relation?.title ||
    relation?.type ||
    relation?.label ||
    relation?.name ||
    relation?.relationshipType ||
    "Relation"
  );
}

function relationDetail(relation) {
  return (
    relation?.detail ||
    relation?.targetLabel ||
    relation?.target ||
    relation?.description ||
    relation?.to ||
    ""
  );
}

export function createRelationRow(relation, doc = globalThis.document) {
  const row = doc.createElement("div");
  row.className = "relation-row";
  const title = doc.createElement("strong");
  title.textContent = relationTitle(relation);
  const detail = doc.createElement("span");
  detail.textContent = relationDetail(relation);
  row.append(title, detail);
  return row;
}

function replaceSectionRows(section, container, rows, rowFactory) {
  if (!container || !section) {
    return 0;
  }
  const renderedRows = rowFactory(rows);
  container.replaceChildren(...renderedRows);
  section.hidden = renderedRows.length === 0;
  return renderedRows.length;
}

export function resetPropertiesBalloonContent(dom = {}) {
  dom.propertiesCoreGrid?.replaceChildren();
  if (dom.propertiesCoreSection) {
    dom.propertiesCoreSection.hidden = true;
  }
  dom.propertiesExtraGrid?.replaceChildren();
  if (dom.propertiesExtraSection) {
    dom.propertiesExtraSection.hidden = true;
  }
  dom.propertiesRelationsList?.replaceChildren();
  if (dom.propertiesRelationsSection) {
    dom.propertiesRelationsSection.hidden = true;
  }
}

export function setPropertiesGraphButtonVisible(dom = {}, visible = false) {
  if (dom.propertiesGraphButton) {
    dom.propertiesGraphButton.hidden = !visible;
  }
}

export function renderPropertiesBalloonView(dom = {}, view = {}) {
  const doc = ownerDocumentForDom(dom);
  if (dom.propertiesTitle && view.title !== undefined) {
    dom.propertiesTitle.textContent = view.title;
  }
  if (dom.propertiesSubtitle && view.subtitle !== undefined) {
    dom.propertiesSubtitle.textContent = view.subtitle;
  }

  const coreRows = Array.isArray(view.coreRows) ? view.coreRows : [];
  const extraRows = Array.isArray(view.extraRows) ? view.extraRows : [];
  const relations = Array.isArray(view.relations) ? view.relations : [];
  const coreCount = replaceSectionRows(
    dom.propertiesCoreSection,
    dom.propertiesCoreGrid,
    coreRows,
    (rows) => createPropertyRows(rows, doc)
  );
  const extraCount = replaceSectionRows(
    dom.propertiesExtraSection,
    dom.propertiesExtraGrid,
    extraRows,
    (rows) => createPropertyRows(rows, doc)
  );
  const relationCount = replaceSectionRows(
    dom.propertiesRelationsSection,
    dom.propertiesRelationsList,
    relations,
    (rows) => rows.map((relation) => createRelationRow(relation, doc))
  );

  const emptyText = view.emptyText ?? view.empty?.text ?? "";
  const emptyVisible = Boolean(
    view.emptyVisible ?? view.empty?.visible ?? (!coreCount && !extraCount && !relationCount)
  );
  if (dom.propertiesEmptyState) {
    dom.propertiesEmptyState.hidden = !emptyVisible;
    dom.propertiesEmptyState.textContent = emptyText;
  }

  setPropertiesGraphButtonVisible(
    dom,
    Boolean(view.graphButtonVisible ?? view.graphButton?.visible)
  );

  return {
    coreRows: coreCount,
    extraRows: extraCount,
    relations: relationCount,
    emptyVisible,
  };
}
