// Generates map tiles and answers structure and search questions off the main thread, see
// src/gui/executor_web.rs and src/gui/finder_web.rs.
import init, { finder_run, worker_run } from "./pkg/cubox.js";

const ready = init();

self.onmessage = async (event) => {
  await ready;
  try {
    // finder jobs are text, tile requests arrays
    if (typeof event.data === "string") {
      self.postMessage(finder_run(event.data));
      return;
    }
    const [rgb, estimates, biomes] = worker_run(event.data);
    self.postMessage([rgb, estimates, biomes], [rgb.buffer, estimates.buffer, biomes.buffer]);
  } catch (error) {
    // the main thread frees this worker again and logs it
    self.postMessage({ error: String(error) });
  }
};
