import { computed, onBeforeUnmount, onMounted, ref, watch, type WritableComputedRef } from 'vue'

/** Measure the fixed iframe viewport, clipped through ancestor documents.
 * IntersectionObserver exposes intersectionRect in this document's coordinates
 * without granting access to the parent's DOM (rootBounds may be null).
 * All measurements are out of flow and never resize the plugin document. */
export function useVisibleDialogViewport(): () => WritableComputedRef<boolean> {
  let observer: IntersectionObserver | undefined
  let target: HTMLDivElement | undefined
  let timer: ReturnType<typeof setTimeout> | undefined
  let stopped = false
  let active = 0
  const pending = new Set<IntersectionObserver>()
  function apply(entry?: IntersectionObserverEntry) {
    const rect = entry?.intersectionRect
    if (!rect || rect.width <= 0 || rect.height <= 0) return
    const style = document.body.style
    style.setProperty('--dialogue-visible-top', `${Math.max(0, rect.top)}px`)
    style.setProperty('--dialogue-visible-left', `${Math.max(0, rect.left)}px`)
    style.setProperty('--dialogue-visible-width', `${rect.width}px`)
    style.setProperty('--dialogue-visible-height', `${rect.height}px`)
  }
  function sample() {
    if (stopped || !observer || !target) return
    // Re-observe: ancestor scrolling with an unchanged intersection ratio does
    // not otherwise produce an entry, even though its position changed.
    observer.unobserve(target)
    observer.observe(target)
    timer = setTimeout(sample, active ? 80 : 200)
  }
  function measureNow(): Promise<void> {
    if (!target || stopped) return Promise.resolve()
    return new Promise(resolve => {
      const once = new IntersectionObserver(entries => {
        apply(entries[entries.length - 1])
        once.disconnect()
        pending.delete(once)
        resolve()
      })
      pending.add(once)
      once.observe(target!)
    })
  }
  onMounted(() => {
    document.body.dataset.dialogueViewport = ''
    target = document.createElement('div')
    target.setAttribute('aria-hidden', 'true')
    target.style.cssText = 'position:fixed;inset:0;pointer-events:none;opacity:0;z-index:-1'
    document.body.appendChild(target)
    observer = new IntersectionObserver(entries => apply(entries[entries.length - 1]))
    sample()
  })
  onBeforeUnmount(() => {
    stopped = true
    if (timer) clearTimeout(timer)
    observer?.disconnect()
    for (const once of pending) once.disconnect()
    pending.clear()
    target?.remove()
    delete document.body.dataset.dialogueViewport
    for (const name of ['top', 'left', 'width', 'height']) document.body.style.removeProperty(`--dialogue-visible-${name}`)
  })
  return () => {
    const open = ref(false)
    let request = 0
    watch(open, value => { active += value ? 1 : -1 }, { flush: 'sync' })
    return computed({
      get: () => open.value,
      set: (value: boolean) => {
        const seq = ++request
        if (!value) { open.value = false; return }
        // Measure after a button has scrolled into view, before native focus
        // restoration can cause the parent to jump to an offscreen dialog.
        void measureNow().then(() => { if (!stopped && seq === request) open.value = true })
      },
    })
  }
}
