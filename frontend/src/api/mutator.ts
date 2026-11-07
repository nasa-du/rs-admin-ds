import axios, { type AxiosRequestConfig } from 'axios'

/**
 * Origin of the API. `VITE_API_BASE_URL` is empty for same-origin
 * deployments; generated OpenAPI paths already carry their full prefix.
 */
const apiBaseUrl = import.meta.env.VITE_API_BASE_URL || undefined

export const apiClient = axios.create({
  baseURL: apiBaseUrl,
  headers: { Accept: 'application/json' },
})

export const customInstance = async <T>(
  config: AxiosRequestConfig,
  options?: AxiosRequestConfig
): Promise<T> => {
  const response = await apiClient.request<T>({
    ...config,
    ...options,
    baseURL: apiBaseUrl,
  })

  return response.data
}
