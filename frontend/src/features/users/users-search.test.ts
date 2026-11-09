import { describe, expect, it } from 'vitest'
import {
  USERS_DEFAULT_SORT,
  clearFiltersPatch,
  listFiltersPatch,
  paginationPatch,
  sortingFromSearch,
  sortingPatch,
  sortFromSorting,
  toListUsersParams,
  usersSearchSchema,
} from './users-search'

const USER_ID = '0190a1b2-0000-7000-8000-000000000001'
const ROLE_ID = '0190a1b2-0000-7000-8000-000000000002'

describe('usersSearchSchema', () => {
  it('parses backend parameter names and normalizes strings', () => {
    const parsed = usersSearchSchema.parse({
      page: 2,
      page_size: 50,
      q: '  alice  ',
      status: 'active',
      role_id: ROLE_ID,
      is_locked: true,
      must_change_password: false,
      sort: '-email',
      user: USER_ID,
    })

    expect(parsed).toEqual({
      page: 2,
      page_size: 50,
      q: 'alice',
      status: 'active',
      role_id: ROLE_ID,
      is_locked: true,
      must_change_password: false,
      sort: '-email',
      user: USER_ID,
    })
  })

  it('accepts string values from a raw query string', () => {
    const parsed = usersSearchSchema.parse({
      page: '3',
      page_size: '100',
      is_locked: 'false',
      must_change_password: 'true',
    })

    expect(parsed.page).toBe(3)
    expect(parsed.page_size).toBe(100)
    expect(parsed.is_locked).toBe(false)
    expect(parsed.must_change_password).toBe(true)
  })

  it('drops invalid values instead of failing', () => {
    const parsed = usersSearchSchema.parse({
      page: 0,
      page_size: 500,
      q: '   ',
      status: 'archived',
      role_id: 'not-a-role',
      is_locked: 'sometimes',
      must_change_password: 1,
      sort: 'roles',
      user: 'not-a-user',
    })

    expect(parsed.page).toBeUndefined()
    expect(parsed.page_size).toBeUndefined()
    expect(parsed.q).toBeUndefined()
    expect(parsed.status).toBeUndefined()
    expect(parsed.role_id).toBeUndefined()
    expect(parsed.is_locked).toBeUndefined()
    expect(parsed.must_change_password).toBeUndefined()
    expect(parsed.sort).toBeUndefined()
    expect(parsed.user).toBeUndefined()
  })
})

describe('toListUsersParams', () => {
  it('omits defaults so the URL and query key stay canonical', () => {
    expect(
      toListUsersParams({
        page: 1,
        page_size: 20,
        sort: USERS_DEFAULT_SORT,
      })
    ).toEqual({
      q: undefined,
      status: undefined,
      role_id: undefined,
      is_locked: undefined,
      must_change_password: undefined,
      page: undefined,
      page_size: undefined,
      sort: undefined,
    })
  })

  it('keeps every non-default filter', () => {
    expect(
      toListUsersParams({
        page: 2,
        page_size: 50,
        q: 'alice',
        status: 'disabled',
        role_id: ROLE_ID,
        is_locked: true,
        must_change_password: false,
        sort: 'email',
      })
    ).toEqual({
      q: 'alice',
      status: 'disabled',
      role_id: ROLE_ID,
      is_locked: true,
      must_change_password: false,
      page: 2,
      page_size: 50,
      sort: 'email',
    })
  })
})

describe('list navigation patches', () => {
  it('resets the page and closes the detail sheet on a filter change', () => {
    expect(listFiltersPatch({ status: 'disabled' })).toEqual({
      page: undefined,
      user: undefined,
      status: 'disabled',
    })
  })

  it('drops the detail sheet on pagination but keeps a non-default page', () => {
    expect(paginationPatch({ pageIndex: 2, pageSize: 20 }, 20)).toEqual({
      page: 3,
      page_size: undefined,
      user: undefined,
    })
  })

  it('returns to page one when the page size changes', () => {
    expect(paginationPatch({ pageIndex: 3, pageSize: 50 }, 20)).toEqual({
      page: undefined,
      page_size: 50,
      user: undefined,
    })
  })

  it('resets the page and sheet on a sort change', () => {
    expect(sortingPatch([{ id: 'username', desc: true }])).toEqual({
      sort: '-username',
      page: undefined,
      user: undefined,
    })
  })

  it('keeps the sort when clearing every filter', () => {
    expect(clearFiltersPatch()).toEqual({
      q: undefined,
      status: undefined,
      role_id: undefined,
      is_locked: undefined,
      must_change_password: undefined,
      page: undefined,
      user: undefined,
    })
  })
})

describe('sort state', () => {
  it('uses the default sort when the URL omits it', () => {
    expect(sortingFromSearch({})).toEqual([{ id: 'created_at', desc: true }])
  })

  it('reads a descending sort from the URL', () => {
    expect(sortingFromSearch({ sort: '-last_login_at' })).toEqual([
      { id: 'last_login_at', desc: true },
    ])
  })

  it('writes the default sort as an omitted search value', () => {
    expect(sortFromSorting([{ id: 'created_at', desc: true }])).toBeUndefined()
  })

  it('writes a non-default sort with its direction prefix', () => {
    expect(sortFromSorting([{ id: 'username', desc: false }])).toBe('username')
    expect(sortFromSorting([{ id: 'username', desc: true }])).toBe('-username')
  })
})
