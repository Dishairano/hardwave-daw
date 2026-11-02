import { describe, it, expect, vi, beforeEach } from 'vitest'

const invokeMock = vi.fn()
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...args: unknown[]) => invokeMock(...args) }))

const { useAccountStore, initialsOf, RECHECK_AFTER_MS } = await import('../accountStore')

const pro = {
  state: 'signedIn', name: 'Dishairano', email: 'dis…@hardwavestudios.com',
  plan: { kind: 'pro', via: 'subscription', until: null },
}

function reset() {
  invokeMock.mockReset()
  useAccountStore.setState({
    available: true, status: null, lastAccount: null, checking: false, busy: false,
    error: null, open: false, lastCheckedAt: 0,
  })
}

describe('the account store', () => {
  beforeEach(reset)

  it('shows what the backend says, and keeps nothing but that', async () => {
    invokeMock.mockResolvedValue(pro)
    await useAccountStore.getState().refresh()
    expect(invokeMock).toHaveBeenCalledWith('account_status')
    expect(useAccountStore.getState().status).toEqual(pro)
    expect(useAccountStore.getState().lastAccount?.name).toBe('Dishairano')
  })

  it('does not ask again within a few seconds of the last check unless told to', async () => {
    invokeMock.mockResolvedValue(pro)
    await useAccountStore.getState().refresh()
    await useAccountStore.getState().refresh()
    expect(invokeMock).toHaveBeenCalledTimes(1)
    useAccountStore.setState({ lastCheckedAt: Date.now() - RECHECK_AFTER_MS - 1 })
    await useAccountStore.getState().refresh()
    expect(invokeMock).toHaveBeenCalledTimes(2)
    await useAccountStore.getState().refresh({ force: true })
    expect(invokeMock).toHaveBeenCalledTimes(3)
  })

  it('keeps showing the account while the site cannot be reached', async () => {
    invokeMock.mockResolvedValueOnce(pro)
    await useAccountStore.getState().refresh({ force: true })
    invokeMock.mockResolvedValueOnce({ state: 'unreachable' })
    await useAccountStore.getState().refresh({ force: true })
    expect(useAccountStore.getState().status).toEqual({ state: 'unreachable' })
    expect(useAccountStore.getState().lastAccount?.name).toBe('Dishairano')
    // A failed call is the same as an unreachable site, not a sign-out.
    invokeMock.mockRejectedValueOnce('network down')
    await useAccountStore.getState().refresh({ force: true })
    expect(useAccountStore.getState().status).toEqual({ state: 'unreachable' })
    expect(useAccountStore.getState().lastAccount?.name).toBe('Dishairano')
  })

  it('a sign-in elsewhere ending shows Sign in again', async () => {
    invokeMock.mockResolvedValueOnce(pro)
    await useAccountStore.getState().refresh({ force: true })
    invokeMock.mockResolvedValueOnce({ state: 'ended' })
    await useAccountStore.getState().refresh({ force: true })
    expect(useAccountStore.getState().lastAccount).toBeNull()
  })

  it('signs in through the backend and hands it the address and password only', async () => {
    invokeMock.mockResolvedValue(pro)
    const ok = await useAccountStore.getState().signIn('dis@hardwavestudios.com', 'hunter2')
    expect(ok).toBe(true)
    expect(invokeMock).toHaveBeenCalledWith('account_sign_in', { email: 'dis@hardwavestudios.com', password: 'hunter2' })
    expect(useAccountStore.getState().status).toEqual(pro)
    // The password is in no state the store keeps.
    expect(JSON.stringify(useAccountStore.getState())).not.toContain('hunter2')
  })

  it('a refused sign-in shows the reason and changes nothing', async () => {
    invokeMock.mockResolvedValueOnce({ state: 'signedOut' })
    await useAccountStore.getState().refresh({ force: true })
    invokeMock.mockRejectedValueOnce('That e-mail address and password do not match an account.')
    const ok = await useAccountStore.getState().signIn('a@b.c', 'wrong')
    expect(ok).toBe(false)
    expect(useAccountStore.getState().error).toBe('That e-mail address and password do not match an account.')
    expect(useAccountStore.getState().status).toEqual({ state: 'signedOut' })
    expect(useAccountStore.getState().busy).toBe(false)
  })

  it('a check that was on its way when someone signed in does not undo it', async () => {
    let answerCheck: (v: unknown) => void = () => {}
    invokeMock.mockImplementationOnce(() => new Promise(r => { answerCheck = r }))
    const check = useAccountStore.getState().refresh({ force: true })
    invokeMock.mockResolvedValueOnce(pro)
    await useAccountStore.getState().signIn('a@b.c', 'pw')
    answerCheck({ state: 'signedOut' })
    await check
    expect(useAccountStore.getState().status).toEqual(pro)
  })

  it('signing out shows Sign in and closes the card', async () => {
    useAccountStore.setState({ status: pro as never, open: true })
    invokeMock.mockResolvedValue({ state: 'signedOut' })
    await useAccountStore.getState().signOut()
    expect(invokeMock).toHaveBeenCalledWith('account_sign_out')
    expect(useAccountStore.getState().status).toEqual({ state: 'signedOut' })
    expect(useAccountStore.getState().lastAccount).toBeNull()
    expect(useAccountStore.getState().open).toBe(false)
  })

  it('on a program without the account commands, there is nothing to show', async () => {
    invokeMock.mockRejectedValue('Command account_status not found')
    await useAccountStore.getState().refresh({ force: true })
    expect(useAccountStore.getState().available).toBe(false)
  })

  it('an answer it cannot read is treated as an unreachable site', async () => {
    invokeMock.mockResolvedValue(null)
    await useAccountStore.getState().refresh({ force: true })
    expect(useAccountStore.getState().status).toEqual({ state: 'unreachable' })
  })

  it('opens only the named pages of the site', () => {
    invokeMock.mockResolvedValue(null)
    useAccountStore.getState().openPage('register')
    expect(invokeMock).toHaveBeenCalledWith('account_open_page', { page: 'register' })
  })
})

describe('initialsOf', () => {
  it('takes the first letter of the first two words', () => {
    expect(initialsOf('Dis Hairano')).toBe('DH')
    expect(initialsOf('dishairano')).toBe('D')
    expect(initialsOf('bob.smith')).toBe('BS')
    expect(initialsOf('d…@hardwavestudios.com')).toBe('DH')
    expect(initialsOf('')).toBe('?')
  })
})
