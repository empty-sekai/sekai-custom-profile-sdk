export type ResourceDescriptor = {
  id: string;
  namespace: string;
  key: string;
  role: string;
  provenance: Record<string, unknown>;
  expectedSize?: { width: number; height: number };
};

export type ResourceSource = Blob | ArrayBuffer | Uint8Array | TexImageSource;

export type ProvidedResource = {
  source: ResourceSource;
};

export type ResourceContext = {
  signal: AbortSignal;
};

/** Resolves a semantic renderer resource through arbitrary caller-owned logic. */
export interface ResourceProvider {
  provide(
    descriptor: ResourceDescriptor,
    context: ResourceContext,
  ): Promise<ProvidedResource | null>;

  cacheIdentity?(descriptor: ResourceDescriptor): string | null;
}

export function profileResourceDescriptors(preparation: Record<string, unknown>): ResourceDescriptor[] {
  const resources = preparation.resources;
  if (!Array.isArray(resources)) throw new Error("Profile preparation did not return a resource list");
  return resources.map((entry) => {
    if (!entry || typeof entry !== "object" || Array.isArray(entry)) {
      throw new Error("Profile resource entry is invalid");
    }
    const request = entry as Record<string, unknown>;
    const resource = request.resource;
    const fallback = request.fallback;
    if (!resource || typeof resource !== "object" || Array.isArray(resource)) {
      throw new Error("Profile resource key is missing");
    }
    if (!fallback || typeof fallback !== "object" || Array.isArray(fallback)) {
      throw new Error("Profile resource fallback metric is missing");
    }
    const { namespace, key } = resource as Record<string, unknown>;
    const { width, height } = fallback as Record<string, unknown>;
    if (typeof namespace !== "string" || typeof key !== "string" || typeof request.lookup_key !== "string") {
      throw new Error("Profile resource identity is invalid");
    }
    if (!isMetric(width) || !isMetric(height)) throw new Error("Profile resource fallback metric is invalid");
    const provenance = request.provenance;
    if (provenance != null && (typeof provenance !== "object" || Array.isArray(provenance))) {
      throw new Error("Profile resource provenance is invalid");
    }
    return {
      id: `${namespace}\0${key}`,
      namespace,
      key,
      role: request.lookup_key,
      provenance: provenance == null ? {} : { ...(provenance as Record<string, unknown>) },
      expectedSize: { width, height },
    };
  });
}

function isMetric(value: unknown): value is number {
  return typeof value === "number" && Number.isFinite(value) && value >= 0;
}
