const DATABASE_NAME = "sekai-custom-profile-sdk-workbench-inputs";
const DATABASE_VERSION = 1;
const STORE_NAME = "inputs";

export class BrowserInputStore {
  async restore() {
    if (!globalThis.indexedDB) return { profileFile: null, fonts: [] };
    const [profile, fonts] = await Promise.all([this.get("profile"), this.get("fonts")]);
    return {
      profileFile: profile ? fileFromRecord(profile) : null,
      fonts: Array.isArray(fonts)
        ? fonts.map((font) => ({ file: fileFromRecord(font), bytes: font.bytes }))
        : [],
    };
  }

  async saveProfile(file) {
    if (!globalThis.indexedDB) return;
    await this.put("profile", {
      name: file.name,
      type: file.type || "application/json",
      lastModified: file.lastModified || Date.now(),
      text: await file.text(),
    });
  }

  async saveFonts(fonts) {
    if (!globalThis.indexedDB) return;
    await this.put("fonts", fonts.map((font) => ({
      name: font.file.name,
      type: font.file.type || "font/otf",
      lastModified: font.file.lastModified || Date.now(),
      bytes: font.bytes.slice(0),
    })));
  }

  async clear() {
    if (!globalThis.indexedDB) return;
    await Promise.all([this.delete("profile"), this.delete("fonts")]);
  }

  async get(key) {
    const database = await openDatabase();
    return requestResult(database.transaction(STORE_NAME, "readonly").objectStore(STORE_NAME).get(key));
  }

  async put(key, value) {
    const database = await openDatabase();
    const transaction = database.transaction(STORE_NAME, "readwrite");
    transaction.objectStore(STORE_NAME).put(value, key);
    await transactionDone(transaction);
  }

  async delete(key) {
    const database = await openDatabase();
    const transaction = database.transaction(STORE_NAME, "readwrite");
    transaction.objectStore(STORE_NAME).delete(key);
    await transactionDone(transaction);
  }
}

function openDatabase() {
  return new Promise((resolve, reject) => {
    const request = indexedDB.open(DATABASE_NAME, DATABASE_VERSION);
    request.onupgradeneeded = () => {
      if (!request.result.objectStoreNames.contains(STORE_NAME)) {
        request.result.createObjectStore(STORE_NAME);
      }
    };
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error ?? new Error("Open workbench input store failed"));
  });
}

function requestResult(request) {
  return new Promise((resolve, reject) => {
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error ?? new Error("Workbench input store request failed"));
  });
}

function transactionDone(transaction) {
  return new Promise((resolve, reject) => {
    transaction.oncomplete = () => resolve();
    transaction.onabort = () => reject(transaction.error ?? new Error("Workbench input transaction aborted"));
    transaction.onerror = () => reject(transaction.error ?? new Error("Workbench input transaction failed"));
  });
}

function fileFromRecord(record) {
  const contents = record.text == null ? [record.bytes] : [record.text];
  if (typeof File === "function") {
    return new File(contents, record.name, {
      type: record.type,
      lastModified: record.lastModified,
    });
  }
  const blob = new Blob(contents, { type: record.type });
  return {
    name: record.name,
    type: record.type,
    size: blob.size,
    lastModified: record.lastModified,
    arrayBuffer: () => blob.arrayBuffer(),
    text: () => blob.text(),
  };
}
