import { describe, expect, it } from 'vitest'
import {
  type GuardUser,
  decideChangePasswordRoute,
  decideProtectedRoute,
  decideSignInRoute,
} from './route-guards'

const USER: GuardUser = {
  must_change_password: false,
  is_super_admin: false,
  permissions: ['users.read'],
}

describe('decideProtectedRoute', () => {
  it('sends an unauthenticated visitor to sign-in with the current path', () => {
    expect(
      decideProtectedRoute({
        user: null,
        authStatus: 'unauthenticated',
        pathname: '/users',
        href: '/users?page=2',
      })
    ).toEqual({ kind: 'sign-in', redirect: '/users?page=2' })
  })

  it('keeps a transient bootstrap failure retryable', () => {
    expect(
      decideProtectedRoute({
        user: null,
        authStatus: 'unknown',
        pathname: '/users',
        href: '/users',
      })
    ).toEqual({ kind: 'bootstrap-error' })
  })

  it('forces a restricted session onto /change-password', () => {
    expect(
      decideProtectedRoute({
        user: { ...USER, must_change_password: true },
        authStatus: 'authenticated',
        pathname: '/users',
        href: '/users?page=2',
      })
    ).toEqual({ kind: 'change-password', redirect: '/users?page=2' })
  })

  it('does not loop when the target already is /change-password', () => {
    expect(
      decideProtectedRoute({
        user: { ...USER, must_change_password: true },
        authStatus: 'authenticated',
        pathname: '/change-password',
        href: '/change-password',
      })
    ).toEqual({ kind: 'allow' })
  })

  it('sends a visitor without the route permission to /403 with the origin', () => {
    expect(
      decideProtectedRoute({
        user: USER,
        authStatus: 'authenticated',
        pathname: '/roles',
        href: '/roles',
      })
    ).toEqual({ kind: 'forbidden', from: '/roles' })
  })

  it('allows a super admin through every route permission check', () => {
    expect(
      decideProtectedRoute({
        user: { ...USER, is_super_admin: true, permissions: [] },
        authStatus: 'authenticated',
        pathname: '/audit-logs',
        href: '/audit-logs',
      })
    ).toEqual({ kind: 'allow' })
  })

  it('allows routes that only require a session', () => {
    expect(
      decideProtectedRoute({
        user: { ...USER, permissions: [] },
        authStatus: 'authenticated',
        pathname: '/settings/account',
        href: '/settings/account',
      })
    ).toEqual({ kind: 'allow' })
  })

  it('checks permission after the forced password change rule', () => {
    expect(
      decideProtectedRoute({
        user: { ...USER, must_change_password: true, permissions: [] },
        authStatus: 'authenticated',
        pathname: '/users',
        href: '/users',
      })
    ).toEqual({ kind: 'change-password', redirect: '/users' })
  })
})

describe('decideChangePasswordRoute', () => {
  it('lets an authenticated user render the page', () => {
    expect(
      decideChangePasswordRoute({
        user: USER,
        authStatus: 'authenticated',
        pathname: '/change-password',
        href: '/change-password?redirect=/users',
      })
    ).toEqual({ kind: 'allow' })
  })

  it('requires a session, preserving the change-password target', () => {
    expect(
      decideChangePasswordRoute({
        user: null,
        authStatus: 'unauthenticated',
        pathname: '/change-password',
        href: '/change-password?redirect=/users',
      })
    ).toEqual({
      kind: 'sign-in',
      redirect: '/change-password?redirect=/users',
    })
  })

  it('keeps a transient bootstrap failure retryable', () => {
    expect(
      decideChangePasswordRoute({
        user: null,
        authStatus: 'unknown',
        pathname: '/change-password',
        href: '/change-password',
      })
    ).toEqual({ kind: 'bootstrap-error' })
  })
})

describe('decideSignInRoute', () => {
  it('renders the form for a visitor without a session', () => {
    expect(decideSignInRoute({ user: null, redirect: '/users' })).toEqual({
      kind: 'allow',
    })
  })

  it('sends an authenticated user to the sanitized target', () => {
    expect(
      decideSignInRoute({ user: USER, redirect: '/users?page=2' })
    ).toEqual({ kind: 'redirect', href: '/users?page=2' })
  })

  it('falls back to the dashboard for an unsafe target', () => {
    expect(
      decideSignInRoute({ user: USER, redirect: '//evil.example' })
    ).toEqual({ kind: 'redirect', href: '/' })
    expect(decideSignInRoute({ user: USER })).toEqual({
      kind: 'redirect',
      href: '/',
    })
  })
})
