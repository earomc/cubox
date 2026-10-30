// Generates map tiles off the main thread, see src/gui/executor_web.rs.
import init, { worker_run } from "./pkg/cubox.js";

const ready = init();

self.onmessage = async (event) => {
  await ready;
  try {
    const [rgb, estimates] = worker_run(event.data);
    self.postMessage([rgb, estimates], [rgb.buffer, estimates.buffer]);
  } catch (error) {
    // the main thread frees this worker again and logs it
    self.postMessage({ error: String(error) });
  }
};
