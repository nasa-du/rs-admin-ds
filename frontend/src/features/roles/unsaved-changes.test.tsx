import { beforeEach, describe, expect, it, vi } from 'vitest'
import { render } from 'vitest-browser-react'
import { userEvent } from 'vitest/browser'
import { UnsavedChangesDialog } from './unsaved-changes'
import { useUnsavedChangesGuard } from './use-unsaved-changes'

const mocks = vi.hoisted(() => ({
  useBlocker: vi.fn(),
}))

vi.mock('@tanstack/react-router', async (importOriginal) => {
  const actual = await importOriginal<typeof import('@tanstack/react-router')>()
  return {
    ...actual,
    useBlocker: (...args: unknown[]) => mocks.useBlocker(...args),
  }
})

function Harness({ dirty }: { dirty: boolean }) {
  const { blocker } = useUnsavedChangesGuard(dirty)

  return (
    <UnsavedChangesDialog
      blocker={blocker}
      title='Discard changes?'
      description='Unsaved changes will be lost.'
      discardLabel='Discard changes'
      continueLabel='Continue editing'
    />
  )
}

describe('useUnsavedChangesGuard', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    mocks.useBlocker.mockReturnValue({
      status: 'idle',
      proceed: undefined,
      reset: undefined,
    })
  })

  it('registers a router blocker and native unload guard while dirty', async () => {
    await render(<Harness dirty />)

    const opts = mocks.useBlocker.mock.calls[0][0]
    expect(opts.enableBeforeUnload).toBe(true)
    expect(opts.disabled).toBe(false)
    expect(opts.withResolver).toBe(true)
    expect(opts.shouldBlockFn()).toBe(true)
  })

  it('lets an explicit navigation through after a successful save', async () => {
    function SaveHarness() {
      const { blocker, allowNextNavigation } = useUnsavedChangesGuard(true)

      return (
        <>
          <button type='button' onClick={allowNextNavigation}>
            Save
          </button>
          <UnsavedChangesDialog
            blocker={blocker}
            title='Discard changes?'
            description='Unsaved changes will be lost.'
            discardLabel='Discard changes'
            continueLabel='Continue editing'
          />
        </>
      )
    }

    const screen = await render(<SaveHarness />)
    const opts = mocks.useBlocker.mock.calls[0][0]

    await userEvent.click(screen.getByRole('button', { name: 'Save' }))
    expect(opts.shouldBlockFn()).toBe(false)
  })

  it('does not register a blocker when the form is clean', async () => {
    await render(<Harness dirty={false} />)

    expect(mocks.useBlocker.mock.calls[0][0].disabled).toBe(true)
    expect(mocks.useBlocker.mock.calls[0][0].enableBeforeUnload).toBe(false)
  })
})

describe('UnsavedChangesDialog', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('continues editing without navigating', async () => {
    const reset = vi.fn()
    const proceed = vi.fn()
    mocks.useBlocker.mockReturnValue({
      status: 'blocked',
      proceed,
      reset,
    })

    const screen = await render(<Harness dirty />)
    await userEvent.click(
      screen.getByRole('button', { name: 'Continue editing' })
    )

    expect(reset).toHaveBeenCalledTimes(1)
    expect(proceed).not.toHaveBeenCalled()
  })

  it('discards changes and proceeds with the blocked navigation', async () => {
    const reset = vi.fn()
    const proceed = vi.fn()
    mocks.useBlocker.mockReturnValue({
      status: 'blocked',
      proceed,
      reset,
    })

    const screen = await render(<Harness dirty />)
    await userEvent.click(
      screen.getByRole('button', { name: 'Discard changes' })
    )

    expect(proceed).toHaveBeenCalledTimes(1)
    expect(reset).not.toHaveBeenCalled()
  })
})
