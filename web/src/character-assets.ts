// Character asset loading (#55): the released Ranger bundle is read from the unpacked
// static files through 3d-lab's bundle loader, once per page. Loading reports byte
// progress; a failure clears the attempt so a retry starts again. The loaded asset is
// shared by every character instance in every session and is never rebuilt.

export interface CharacterProgress {
  loadedBytes: number;
  totalBytes: number;
}

export interface CharacterBundle {
  release: string;
  key: string;
  variant: string;
}

export interface CharacterAssetsOptions<Asset> {
  bundle: CharacterBundle;
  // URL of the directory that holds `<release>/manifest.json` (Vite's base + "characters/").
  baseUrl: string;
  fetch: (url: string) => Promise<{
    ok: boolean;
    status: number;
    arrayBuffer(): Promise<ArrayBuffer>;
    json(): Promise<unknown>;
  }>;
  adapt: (
    source: {
      manifest: unknown;
      key: string;
      variant: string;
      readResource(path: string): Promise<ArrayBuffer>;
    },
    options: { onProgress(progress: CharacterProgress): void },
  ) => Promise<Asset>;
}

export function createCharacterAssets<Asset>(options: CharacterAssetsOptions<Asset>) {
  const { bundle } = options;
  const root = `${options.baseUrl}${bundle.release}/`;
  let asset: Asset | null = null;
  let loading: Promise<Asset> | null = null;
  let progress: CharacterProgress | null = null;
  const listeners = new Set<() => void>();
  const notify = () => {
    for (const listener of listeners) listener();
  };
  const request = async (path: string) => {
    const response = await options.fetch(`${root}${path}`);
    if (!response.ok) throw new Error(`Character file ${path} is unavailable (${response.status})`);
    return response;
  };

  const load = (): Promise<Asset> => {
    if (asset) return Promise.resolve(asset);
    if (loading) return loading;
    progress = { loadedBytes: 0, totalBytes: 0 };
    notify();
    const attempt = (async () => {
      const manifest = await (await request("manifest.json")).json();
      return options.adapt(
        {
          manifest,
          key: bundle.key,
          variant: bundle.variant,
          readResource: async (path) => (await request(path)).arrayBuffer(),
        },
        {
          onProgress: (next) => {
            if (loading !== attempt) return;
            progress = { ...next };
            notify();
          },
        },
      );
    })().then(
      (loaded) => {
        asset = loaded;
        loading = null;
        progress = null;
        notify();
        return loaded;
      },
      (error) => {
        loading = null;
        progress = null;
        notify();
        throw new Error(
          `Character assets failed to load: ${error instanceof Error ? error.message : error}`,
          {
            cause: error,
          },
        );
      },
    );
    loading = attempt;
    return attempt;
  };

  return {
    load,
    // The loaded asset, or null while unloaded, loading or failed.
    get: () => asset,
    // Byte progress of the load in flight, or null.
    getProgress: () => progress,
    subscribe(listener: () => void) {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
  };
}

export type CharacterAssets<Asset> = ReturnType<typeof createCharacterAssets<Asset>>;
