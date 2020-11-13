#!/usr/bin/env bash

set -e

target=$1
package_arch=klkgw

package_name=helium_gateway
package_version="0.0.2"

machine="$(cut -d '-' -f1 <<<"${target}")"
package_src=package/keros

# make base folder
build_dir="target/keros/${machine}"
mkdir -p "${build_dir}"
echo 2.0 > "${build_dir}/debian-binary"

# install data files
cp -R "${package_src}/data" "${build_dir}"

# install binary
mkdir -p "${build_dir}/data/usr/bin"
cp "target/${target}/release/helium_gateway" "${build_dir}/data/usr/bin"

# install control files
mkdir -p "${build_dir}/control"
export target machine package_version package_arch
for control_file in control preinst postinst prerm; do
    envsubst < "${package_src}/control/${control_file}" > "${build_dir}/control/${control_file}"
    chmod +x "${build_dir}/control/${control_file}"
done



# package together
pushd "${build_dir}/control" > /dev/null
tar --numeric-owner --gid=0 --uid=0 -czf ../control.tar.gz ./*
popd > /dev/null

pushd "${build_dir}/data" > /dev/null
tar --numeric-owner --gid=0 --uid=0 -czf ../data.tar.gz ./*
popd > /dev/null

pushd "${build_dir}" > /dev/null
tar --numeric-owner --gid=0 --uid=0 -cf "../${package_name}_${package_version}.${machine}.ipk" ./debian-binary ./data.tar.gz ./control.tar.gz 
popd > /dev/null
