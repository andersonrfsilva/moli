//! Realm-local Fetch stream helpers, compiled only when a JS stream is consumed
//! or cloned. Neither initialization state nor callbacks are author properties.

use crate::util::{get_private_value, set_private_value, v8_string};

const HELPERS_SLOT: &str = "__moliFetchBodyStreamHelpers";

pub(super) enum BodyStreamHelper {
    Consume,
    Tee,
}

pub(super) fn get<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    helper: BodyStreamHelper,
) -> Option<v8::Local<'s, v8::Function>> {
    let global = scope.get_current_context().global(scope);
    let helpers = if let Some(value) = get_private_value(scope, global, HELPERS_SLOT) {
        v8::Local::<v8::Array>::try_from(value).ok()?
    } else {
        let source = v8_string(scope, SOURCE)?;
        let script = v8::Script::compile(scope, source, None)?;
        let value = crate::script_execution::execute_compiled_script(scope, script)?;
        let helpers = v8::Local::<v8::Array>::try_from(value).ok()?;
        set_private_value(scope, global, HELPERS_SLOT, helpers.into());
        helpers
    };
    let value = helpers.get_index(scope, helper as u32)?;
    v8::Local::<v8::Function>::try_from(value).ok()
}

const SOURCE: &str = r#"
(() => {
  function uint8ArrayChunkBytes(chunk) {
    if (Object.prototype.toString.call(chunk) !== "[object Uint8Array]") {
      throw new TypeError("ReadableStream body chunks must be Uint8Array");
    }
    return chunk;
  }

  async function consumeReadableStreamBody(stream, kind, mimeType, onChunk) {
    const reader = stream.getReader();
    const chunks = [];
    let total = 0;
    try {
      for (;;) {
        const result = await reader.read();
        if (result.done) break;
        const chunk = uint8ArrayChunkBytes(result.value);
        if (typeof onChunk === "function") onChunk(chunk);
        chunks.push(chunk);
        total += chunk.byteLength;
      }
    } finally {
      try {
        reader.releaseLock();
      } catch (_) {}
    }

    const bytes = new Uint8Array(total);
    let offset = 0;
    for (const chunk of chunks) {
      bytes.set(chunk, offset);
      offset += chunk.byteLength;
    }

    if (kind === "arrayBuffer") return bytes.buffer;
    if (kind === "bytes") return bytes;
    if (kind === "text") return new TextDecoder().decode(bytes);
    if (kind === "json") return JSON.parse(new TextDecoder().decode(bytes));
    if (kind === "blob") return new Blob([bytes], { type: mimeType || "" });
    if (kind === "formData") {
      return new Response(bytes, {
        headers: [["Content-Type", mimeType || ""]]
      }).formData();
    }
    throw new TypeError("Unsupported body consumption kind");
  }

  function teeReadableStreamBody(stream) {
    if (typeof stream.tee === "function") {
      return stream.tee();
    }

    const reader = stream.getReader();
    const controllers = [null, null];
    let reading = false;
    let closed = false;
    let errored = false;
    let storedError;

    function closeAll() {
      if (closed) return;
      closed = true;
      for (const controller of controllers) {
        if (controller) controller.close();
      }
    }

    function errorAll(error) {
      if (errored) return;
      errored = true;
      storedError = error;
      for (const controller of controllers) {
        if (controller && typeof controller.error === "function") {
          controller.error(error);
        }
      }
    }

    function pump() {
      if (reading || closed || errored || !controllers[0] || !controllers[1]) return;
      reading = true;
      reader.read().then(
        result => {
          reading = false;
          if (result.done) {
            closeAll();
            return;
          }
          try {
            controllers[0].enqueue(result.value);
            controllers[1].enqueue(result.value);
          } catch (error) {
            errorAll(error);
            return;
          }
          if (controllers[0].desiredSize > 0 || controllers[1].desiredSize > 0) {
            pump();
          }
        },
        error => {
          reading = false;
          errorAll(error);
        }
      );
    }

    function branch(index) {
      return new ReadableStream({
        start(controller) {
          controllers[index] = controller;
          if (errored) {
            if (typeof controller.error === "function") controller.error(storedError);
          } else if (closed) {
            controller.close();
          } else {
            pump();
          }
        },
        pull() {
          pump();
        },
        cancel() {}
      });
    }

    return [branch(0), branch(1)];
  }

  return [consumeReadableStreamBody, teeReadableStreamBody];
})();
"#;
