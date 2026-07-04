import { sampleClassification } from "./fixtures";

export function jsonResponse(body: unknown, init?: ResponseInit) {
  return Promise.resolve(
    new Response(JSON.stringify(body), {
      status: 200,
      headers: { "content-type": "application/json" },
      ...init
    })
  );
}

export function classificationResponse() {
  return jsonResponse({ classification: sampleClassification });
}
