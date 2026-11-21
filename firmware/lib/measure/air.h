// OpenSights measurement core - the speed of sound in air (see ../../MODEL.md once it
// exists, and ../../../PLAN.md D8). Portable: no Arduino, no IDF; also built on the PC.
//
// Cramer (1993), "The variation of the specific heat ratio and the speed of sound in air
// with temperature, pressure, humidity, and CO2 concentration", JASA 93, 2510. Valid for
// 0-30 degC and 75-102 kPa; used a little beyond that. Water vapour from the relative
// humidity through the saturation vapour pressure and the enhancement factor (Davis 1992,
// as Cramer uses them). CO2 is fixed at 400 ppm (its effect: about 0.01 m/s per 100 ppm).
//
// Reference values (101 325 Pa): 0 degC dry 331.45 m/s; 20 degC dry 343.36; 20 degC 50 %
// 343.99; 18 degC 50 % 342.74; 25 degC 50 % 347.13. Near 18 degC: +0.62 m/s per degC,
// +0.011 m/s per % RH.
#pragma once
#include <math.h>

namespace measure {

// Speed of sound in m/s; tC in degC, rh in % (0..100), p in Pa. Double inside: called a
// few times a minute, not per sample.
inline double soundSpeedMs(double tC, double rh, double p) {
  const double K = tC + 273.15;
  const double psv = exp(1.2378847e-5 * K * K - 1.9121316e-2 * K + 33.93711047 - 6.3431645e3 / K);
  const double f = 1.00062 + 3.14e-8 * p + 5.6e-7 * tC * tC;
  const double xw = rh / 100.0 * f * psv / p;   // mole fraction of water vapour
  const double xc = 0.0004;                     // mole fraction of CO2
  const double t = tC;
  return 331.5024 + 0.603055 * t - 0.000528 * t * t
       + (51.471935 + 0.1495874 * t - 0.000782 * t * t) * xw
       + (-1.82e-7 + 3.73e-8 * t - 2.93e-10 * t * t) * p
       + (-85.20931 - 0.228525 * t + 5.91e-5 * t * t) * xc
       - 2.835149 * xw * xw - 2.15e-13 * p * p + 29.179762 * xc * xc + 0.000486 * xw * p * xc;
}

}  // namespace measure
