interface FetchItemsParams {
  buildAuthedUrl: (pathname: string, options?: Record<string, unknown>) => URL;
  searchQuery: string;
}

interface PatchItemMetadataParams {
  buildAuthedUrl: (pathname: string, options?: Record<string, unknown>) => URL;
  itemId: unknown;
  payload: {
    title: string;
    message: string;
  };
  options?: {
    keepalive?: boolean;
  };
}

interface CreateItemParams {
  buildAuthedUrl: (pathname: string, options?: Record<string, unknown>) => URL;
  payload: {
    url: string;
    title: string;
    message: string;
    forceRefresh: boolean;
  };
}

interface DeleteItemParams {
  buildAuthedUrl: (pathname: string, options?: Record<string, unknown>) => URL;
  itemId: unknown;
}

interface ApplyLocalItemMetadataParams {
  items: unknown;
  selectedItem: unknown;
  itemId: unknown;
  nextTitle: unknown;
  nextMessage: unknown;
}

interface ApplyLocalItemMetadataResult {
  items: unknown[];
  selectedItem: unknown;
  updated: boolean;
}

export interface BoardLegacyItemUtils {
  fetchItems(params: FetchItemsParams): Promise<{ items: unknown[]; total: number }>;
  patchItemMetadata(params: PatchItemMetadataParams): Promise<void>;
  createItem(params: CreateItemParams): Promise<Record<string, unknown>>;
  deleteItem(params: DeleteItemParams): Promise<void>;
  applyLocalItemMetadata(params: ApplyLocalItemMetadataParams): ApplyLocalItemMetadataResult;
}

function toRecord(value: unknown): Record<string, unknown> {
  return typeof value === 'object' && value !== null ? (value as Record<string, unknown>) : {};
}

function toTrimmedString(value: unknown): string {
  return `${value || ''}`.trim();
}

export function createBoardLegacyItemUtils(): BoardLegacyItemUtils {
  async function fetchItems(params: FetchItemsParams): Promise<{ items: unknown[]; total: number }> {
    const endpoint = params.buildAuthedUrl('/overlay/meme-board/items', {
      limit: 150,
      offset: 0,
      q: params.searchQuery
    });

    const response = await fetch(endpoint.toString(), {
      method: 'GET',
      cache: 'no-store'
    });
    const payload = await response.json().catch(() => null);

    if (!response.ok) {
      const payloadRecord = toRecord(payload);
      const payloadError = toTrimmedString(payloadRecord.error);
      const message = payloadError ? `Chargement board impossible: ${payloadError}` : 'Chargement board impossible.';
      throw new Error(message);
    }

    const payloadRecord = toRecord(payload);
    const items = Array.isArray(payloadRecord.items) ? payloadRecord.items : [];
    const total = typeof payloadRecord.total === 'number' ? payloadRecord.total : items.length;

    return {
      items,
      total
    };
  }

  async function patchItemMetadata(params: PatchItemMetadataParams): Promise<void> {
    const endpoint = params.buildAuthedUrl(`/overlay/meme-board/items/${params.itemId}`);
    const response = await fetch(endpoint.toString(), {
      method: 'PATCH',
      headers: {
        'Content-Type': 'application/json'
      },
      body: JSON.stringify(params.payload),
      keepalive: params.options?.keepalive === true
    });

    if (!response.ok) {
      const body = await response.json().catch(() => null);
      const bodyRecord = toRecord(body);
      const bodyError = toTrimmedString(bodyRecord.error);
      throw new Error(bodyError || `HTTP_${response.status}`);
    }
  }

  async function createItem(params: CreateItemParams): Promise<Record<string, unknown>> {
    const endpoint = params.buildAuthedUrl('/overlay/meme-board/items');
    const response = await fetch(endpoint.toString(), {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json'
      },
      body: JSON.stringify(params.payload)
    });
    const body = await response.json().catch(() => null);
    const bodyRecord = toRecord(body);

    if (!response.ok) {
      const bodyMessage = toTrimmedString(bodyRecord.message);
      const bodyError = toTrimmedString(bodyRecord.error);
      const reason = bodyMessage || bodyError || `HTTP_${response.status}`;
      throw new Error(reason);
    }

    return bodyRecord;
  }

  async function deleteItem(params: DeleteItemParams): Promise<void> {
    const endpoint = params.buildAuthedUrl(`/overlay/meme-board/items/${params.itemId}`);
    const response = await fetch(endpoint.toString(), {
      method: 'DELETE'
    });

    if (!response.ok) {
      const payload = await response.json().catch(() => null);
      const payloadRecord = toRecord(payload);
      const payloadError = toTrimmedString(payloadRecord.error);
      throw new Error(payloadError || `HTTP_${response.status}`);
    }
  }

  function applyLocalItemMetadata(params: ApplyLocalItemMetadataParams): ApplyLocalItemMetadataResult {
    const itemId = toTrimmedString(params.itemId);
    const nextTitle = toTrimmedString(params.nextTitle);
    const nextMessage = toTrimmedString(params.nextMessage);

    let updated = false;
    const nextItems = (Array.isArray(params.items) ? params.items : []).map((entry) => {
      const entryRecord = toRecord(entry);
      if (toTrimmedString(entryRecord.id) !== itemId) {
        return entry;
      }

      updated = true;
      return {
        ...entryRecord,
        title: nextTitle,
        message: nextMessage
      };
    });

    const selectedItemRecord = toRecord(params.selectedItem);
    let nextSelectedItem = params.selectedItem;
    if (toTrimmedString(selectedItemRecord.id) === itemId) {
      nextSelectedItem = {
        ...selectedItemRecord,
        title: nextTitle,
        message: nextMessage
      };
      updated = true;
    }

    return {
      items: nextItems,
      selectedItem: nextSelectedItem,
      updated
    };
  }

  return {
    fetchItems,
    patchItemMetadata,
    createItem,
    deleteItem,
    applyLocalItemMetadata
  };
}
