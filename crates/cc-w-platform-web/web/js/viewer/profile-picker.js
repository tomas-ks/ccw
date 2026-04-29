function normalizedProfileName(value) {
  return String(value || "").trim();
}

function normalizeProfileDescriptor(profile) {
  const name = normalizedProfileName(profile?.name || profile?.id || profile);
  if (!name) {
    return null;
  }
  return {
    name,
    label: String(profile?.label || name).trim() || name,
    experimental: Boolean(profile?.experimental),
  };
}

function profileDescriptorsFromState(state) {
  const profiles =
    state?.committedViewerState?.availableRenderProfiles ||
    state?.committedViewerState?.available_render_profiles ||
    [];
  return Array.isArray(profiles)
    ? profiles.map(normalizeProfileDescriptor).filter(Boolean)
    : [];
}

function currentProfileFromState(state) {
  return normalizedProfileName(
    state?.committedViewerState?.renderProfile ||
      state?.committedViewerState?.render_profile
  );
}

function currentProfileFromViewer(viewer) {
  if (!viewer || typeof viewer.currentProfile !== "function") {
    return "";
  }
  try {
    return normalizedProfileName(viewer.currentProfile());
  } catch (_error) {
    return "";
  }
}

function profileDescriptorsFromViewer(viewer) {
  if (!viewer || typeof viewer.profiles !== "function") {
    return [];
  }
  try {
    const profiles = viewer.profiles();
    return Array.isArray(profiles)
      ? profiles.map(normalizeProfileDescriptor).filter(Boolean)
      : [];
  } catch (_error) {
    return [];
  }
}

function profileEntriesForRender({ state, viewer }) {
  const currentProfile = currentProfileFromState(state) || currentProfileFromViewer(viewer);
  const source = profileDescriptorsFromState(state);
  const profiles = source.length ? source : profileDescriptorsFromViewer(viewer);
  const seen = new Set();
  const entries = [];
  for (const profile of profiles) {
    if (seen.has(profile.name)) {
      continue;
    }
    if (profile.experimental && profile.name !== currentProfile) {
      continue;
    }
    seen.add(profile.name);
    entries.push(profile);
  }
  if (currentProfile && !seen.has(currentProfile)) {
    entries.push({
      name: currentProfile,
      label: currentProfile,
      experimental: false,
    });
  }
  return entries;
}

function optionSignature(entries) {
  return entries
    .map(
      (entry) =>
        `${entry.name}\u0000${entry.label}\u0000${entry.experimental ? "1" : "0"}`
    )
    .join("\u0001");
}

function createOption(doc, profile) {
  const option = doc.createElement("option");
  option.value = profile.name;
  option.textContent = profile.label;
  if (profile.experimental) {
    option.dataset.experimental = "true";
  }
  return option;
}

export function profilePickerSelection({ state, viewer = null, picker = null } = {}) {
  const currentProfile = currentProfileFromState(state) || currentProfileFromViewer(viewer);
  if (
    currentProfile &&
    (!picker ||
      Array.from(picker.options || []).some((option) => option.value === currentProfile))
  ) {
    return currentProfile;
  }
  return normalizedProfileName(picker?.value);
}

export function createProfilePickerController({
  viewer,
  appStateStore,
  document: doc = globalThis.document,
  picker = doc?.getElementById?.("render-profile-picker") || null,
  subscribe = true,
  onProfileRequested = null,
} = {}) {
  if (!viewer || !appStateStore || !picker) {
    return null;
  }

  let lastOptionSignature = "";
  let rendering = false;

  const render = (state = appStateStore.getState()) => {
    const entries = profileEntriesForRender({ state, viewer });
    const signature = optionSignature(entries);
    rendering = true;
    try {
      if (signature !== lastOptionSignature) {
        picker.replaceChildren(...entries.map((entry) => createOption(doc, entry)));
        lastOptionSignature = signature;
      }

      const selected = profilePickerSelection({ state, viewer, picker });
      if (selected && picker.value !== selected) {
        picker.value = selected;
      }
      picker.disabled = entries.length === 0;
      return selected;
    } finally {
      rendering = false;
    }
  };

  const renderSafely = (state = appStateStore.getState()) => {
    try {
      return render(state);
    } catch (error) {
      console.error("render profile picker render failed", error);
      return "";
    }
  };

  const onChange = () => {
    if (rendering) {
      return;
    }
    const profile = normalizedProfileName(picker.value);
    if (!profile) {
      renderSafely();
      return;
    }
    if (typeof onProfileRequested === "function") {
      onProfileRequested(profile);
    }
    try {
      viewer.setProfile(profile);
    } catch (error) {
      console.error("viewer render profile change failed", error);
      renderSafely();
    }
  };

  picker.addEventListener("change", onChange);

  const unsubscribe = subscribe
    ? appStateStore.subscribe((state) => {
        renderSafely(state);
      })
    : null;

  renderSafely();

  return {
    picker,
    render,
    renderSafely,
    selectedProfile: () =>
      profilePickerSelection({
        state: appStateStore.getState(),
        viewer,
        picker,
      }),
    profiles: () =>
      profileEntriesForRender({ state: appStateStore.getState(), viewer }),
    dispose: () => {
      picker.removeEventListener("change", onChange);
      unsubscribe?.();
    },
  };
}

export function installProfilePicker(viewer, appStateStore, options = {}) {
  return createProfilePickerController({
    ...options,
    viewer,
    appStateStore,
  });
}
