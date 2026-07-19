import {
  RendererAuthoringDocument,
  RendererWorkerClient,
  type RendererWorkerClientOptions,
} from "./worker-client.js";
import type { AuthoringCheckpoint } from "./types/authoring.js";

/** Worker-backed authoring runtime for game-compatible custom-profile documents. */
export class BrowserAuthoringClient {
  private destroyed = false;

  private constructor(private readonly worker: RendererWorkerClient) {}

  static async create(options: RendererWorkerClientOptions = {}): Promise<BrowserAuthoringClient> {
    return new BrowserAuthoringClient(await RendererWorkerClient.create(options));
  }

  async createBlank(): Promise<RendererAuthoringDocument> {
    this.assertAlive();
    return this.worker.createAuthoringDocument();
  }

  async importProfile(profile: unknown): Promise<RendererAuthoringDocument> {
    this.assertAlive();
    return this.worker.createAuthoringDocument(profile);
  }

  async restoreCheckpoint(checkpoint: AuthoringCheckpoint): Promise<RendererAuthoringDocument> {
    this.assertAlive();
    return this.worker.restoreAuthoringDocument(checkpoint);
  }

  destroy(): void {
    if (this.destroyed) return;
    this.destroyed = true;
    this.worker.terminate();
  }

  private assertAlive(): void {
    if (this.destroyed) throw new Error("Browser authoring client is destroyed");
  }
}
