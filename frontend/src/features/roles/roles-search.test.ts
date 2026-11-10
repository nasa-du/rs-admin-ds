import { describe, expect, it } from 'vitest'
import {
  ROLES_DEFAULT_SORT,
  clearFiltersPatch,
  hasActiveFilters,
  listFiltersPatch,
  paginationPatch,
  rolesSearchSchema,
  sortingFromSearch,
  sortingPatch,
  sortFromSorting,
  toListRolesParams,
} from './roles-search'

describe('rolesSearchSchema', () => {
  it('parses backend parameter names and normalizes strings', () => {
    const parsed = rolesSearchSchema.parse({
      page: 2,
      page_size: 50,
      q: '  support  ',
      is_builtin: false,
      sort: '-updated_at',
    })

    expect(parsed).toEqual({
      page: 2,
      page_size: 50,
      q: 'support',
      is_builtin: false,
      sort: '-updated_at',
    })
  })

  it('accepts string values from a raw query string', () => {
    const parsed = rolesSearchSchema.parse({
      page: '3',
      page_size: '100',
      is_builtin: 'true',
    })

    expect(parsed.page).toBe(3)
    expect(parsed.page_size).toBe(100)
    expect(parsed.is_builtin).toBe(true)
  })

  it('drops invalid values instead of failing', () => {
    const parsed = rolesSearchSchema.parse({
      page: 0,
      page_size: 201,
      q: '   ',
      is_builtin: 'sometimes',
      sort: 'description',
    })

    expect(parsed.page).toBeUndefined()
    expect(parsed.page_size).toBeUndefined()
    expect(parsed.q).toBeUndefined()
    expect(parsed.is_builtin).toBeUndefined()
    expect(parsed.sort).toBeUndefined()
  })
})

describe('toListRolesParams', () => {
  it('omits defaults so the URL and query key stay canonical', () => {
    expect(
      toListRolesParams({
        page: 1,
        page_size: 20,
        sort: ROLES_DEFAULT_SORT,
      })
    ).toEqual({
      q: undefined,
      is_builtin: undefined,
      page: undefined,
      page_size: undefined,
      sort: undefined,
    })
  })

  it('keeps every non-default filter', () => {
    expect(
      toListRolesParams({
        page: 2,
        page_size: 50,
        q: 'support',
        is_builtin: false,
        sort: '-created_at',
      })
    ).toEqual({
      q: 'support',
      is_builtin: false,
      page: 2,
      page_size: 50,
      sort: '-created_at',
    })
  })
})

describe('list navigation patches', () => {
  it('resets the page on a filter change', () => {
    expect(listFiltersPatch({ is_builtin: true })).toEqual({
      page: undefined,
      is_builtin: true,
    })
  })

  it('keeps a non-default page and drops it when the page size changes', () => {
    expect(paginationPatch({ pageIndex: 2, pageSize: 20 }, 20)).toEqual({
      page: 3,
      page_size: undefined,
    })
    expect(paginationPatch({ pageIndex: 3, pageSize: 50 }, 20)).toEqual({
      page: undefined,
      page_size: 50,
    })
  })

  it('resets the page on a sort change', () => {
    expect(sortingPatch([{ id: 'name', desc: true }])).toEqual({
      sort: '-name',
      page: undefined,
    })
  })

  it('clears filters and returns to the first page', () => {
    expect(clearFiltersPatch()).toEqual({
      q: undefined,
      is_builtin: undefined,
      page: undefined,
    })
  })
})

describe('filter and sort state', () => {
  it('reports active filters', () => {
    expect(hasActiveFilters({})).toBe(false)
    expect(hasActiveFilters({ q: 'support' })).toBe(true)
    expect(hasActiveFilters({ is_builtin: false })).toBe(true)
  })

  it('uses the default sort when the URL omits it', () => {
    expect(sortingFromSearch({})).toEqual([{ id: 'name', desc: false }])
  })

  it('reads a descending sort from the URL', () => {
    expect(sortingFromSearch({ sort: '-updated_at' })).toEqual([
      { id: 'updated_at', desc: true },
    ])
  })

  it('writes the default sort as an omitted search value', () => {
    expect(sortFromSorting([{ id: 'name', desc: false }])).toBeUndefined()
  })

  it('writes a non-default sort with its direction prefix', () => {
    expect(sortFromSorting([{ id: 'created_at', desc: false }])).toBe(
      'created_at'
    )
    expect(sortFromSorting([{ id: 'created_at', desc: true }])).toBe(
      '-created_at'
    )
  })
})
